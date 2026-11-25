//! `--watch`: compile, then compile again after every change that can change the result.
//!
//! The watcher never compiles in its own process. Each compile runs this binary again without the
//! watch flags, with `--quiet --summary-json`, in its own process group. A compile that stops
//! (`stop_compile()` ends the process) leaves the watcher running, and a Ctrl+C in the terminal
//! reaches the watcher only, which then stops the compile itself.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode, CreateKind, MetadataKind, ModifyKind, RemoveKind};
use notify::{Event, EventKind, RecursiveMode, Watcher};

use crate::watch_set::{self, WatchSet};

const DEBOUNCE: Duration = Duration::from_millis(300);
const METADATA_CHECK: Duration = Duration::from_secs(2);
const STOP_GRACE: Duration = Duration::from_secs(5);
/// A terminal sends Ctrl+C to every process of its foreground group, and `bin/magento` forwards
/// it too, so signals this close together count as one stop request.
const SAME_STOP: Duration = Duration::from_secs(1);
const CLEAN_TIMEOUT: Duration = Duration::from_secs(60);
const CLEAN_RETRY_FIRST: Duration = Duration::from_secs(2);
const CLEAN_RETRY_MAX: Duration = Duration::from_secs(60);
const POLL_DEFAULT: Duration = Duration::from_millis(1000);
const PIPE_DRAIN: Duration = Duration::from_millis(500);

pub struct Options {
    pub magento_root: PathBuf,
    pub generated_root: PathBuf,
    pub php: String,
    pub poll: Option<Duration>,
    pub verbose: bool,
    /// Stop as on SIGTERM when the process that started the watcher ends.
    pub exit_with_parent: bool,
}

/// Arguments of this process that a compile child gets, without the watch flags and without the
/// output flags the watcher sets itself.
pub fn child_arguments(
    arguments: impl IntoIterator<Item = OsString>,
    verbose: bool,
) -> Vec<OsString> {
    let mut kept: Vec<OsString> = arguments
        .into_iter()
        .filter(|argument| {
            let text = argument.to_string_lossy();
            !matches!(
                text.as_ref(),
                "--watch"
                    | "--watch-poll"
                    | "--watch-exit-with-parent"
                    | "--quiet"
                    | "-q"
                    | "--summary-json"
            ) && !text.starts_with("--watch-poll=")
        })
        .collect();
    if !verbose {
        kept.push("--quiet".into());
    }
    kept.push("--summary-json".into());
    kept
}

/// Signals the watcher received, counted, with the last signal number.
#[derive(Clone, Default)]
struct Received {
    count: Arc<AtomicUsize>,
    last: Arc<AtomicUsize>,
}

impl Received {
    fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    fn exit_code(&self) -> i32 {
        128 + self.last.load(Ordering::SeqCst) as i32
    }
}

#[cfg(unix)]
fn listen_for_signals() -> Received {
    use signal_hook::consts::{SIGINT, SIGTERM};
    let received = Received::default();
    match signal_hook::iterator::Signals::new([SIGINT, SIGTERM]) {
        Ok(mut signals) => {
            let shared = received.clone();
            std::thread::spawn(move || {
                for signal in signals.forever() {
                    shared.last.store(signal as usize, Ordering::SeqCst);
                    shared.count.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
        Err(e) => eprintln!("warning: could not listen for SIGINT/SIGTERM: {e}"),
    }
    received
}

#[cfg(not(unix))]
fn listen_for_signals() -> Received {
    Received::default()
}

/// A wrapper such as `bin/magento` can end without passing its signal on, for example when PHP
/// has no `pcntl`. The watcher then counts the end of its parent as SIGTERM.
#[cfg(unix)]
fn stop_when_parent_ends(received: &Received) {
    let parent = unsafe { libc::getppid() };
    let shared = received.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(200));
        if unsafe { libc::getppid() } != parent {
            shared.last.store(libc::SIGTERM as usize, Ordering::SeqCst);
            shared.count.fetch_add(1, Ordering::SeqCst);
            return;
        }
    });
}

#[cfg(not(unix))]
fn stop_when_parent_ends(_received: &Received) {}

type Events = mpsc::Receiver<notify::Result<Event>>;

pub fn run(options: Options) -> ! {
    let received = listen_for_signals();
    if options.exit_with_parent {
        stop_when_parent_ends(&received);
    }
    let child_args = child_arguments(std::env::args_os().skip(1), options.verbose);
    let (sender, events): (mpsc::Sender<notify::Result<Event>>, Events) = mpsc::channel();

    let mut set = watch_set::resolve(&options.magento_root);
    let (mut watcher, mode) = start_watcher(&options, sender, &set);
    let file_events = mode == "file events";
    eprintln!(
        "{} watching {} folders ({mode}); Ctrl+C to stop",
        clock(),
        set.folder_count()
    );
    if options.verbose {
        for folder in &set.recursive {
            eprintln!("  {}", folder.display());
        }
        for filtered in &set.filtered {
            eprintln!(
                "  {} ({})",
                filtered.folder.display(),
                filtered.names.join(", ")
            );
        }
    }

    let global = options.generated_root.join("metadata/global.php");
    let mut pending: Vec<PathBuf> = Vec::new();
    let mut new_folders: Vec<PathBuf> = Vec::new();
    let mut last_event = Instant::now();
    let mut compile_now: Option<String> = Some("start".into());
    let mut config_cache = ConfigCacheClean::default();
    let mut check_metadata = false;
    let mut last_check = Instant::now();

    loop {
        if received.count() > 0 {
            std::process::exit(received.exit_code());
        }

        let due = !pending.is_empty() && last_event.elapsed() >= DEBOUNCE;
        if compile_now.is_some() || due {
            let triggers = std::mem::take(&mut pending);
            // Nothing changed for the debounce time, so a copied tree is complete by now.
            for folder in new_folders.drain(..) {
                if folder.is_dir() {
                    let _ = watcher.watch(&folder, RecursiveMode::Recursive);
                }
            }
            let trigger = compile_now
                .take()
                .unwrap_or_else(|| describe(&triggers, &options));
            if triggers
                .iter()
                .any(|path| watch_set::is_module_list_change(path, &options.magento_root))
            {
                config_cache.request();
            }
            if triggers
                .iter()
                .any(|path| watch_set::is_composer_change(path, &options.magento_root))
            {
                let next = watch_set::resolve(&options.magento_root);
                replace_watches(watcher.as_mut(), &set, &next);
                set = next;
            }
            let ok = compile(&options, &child_args, &received, &trigger);
            if ok {
                config_cache.run_if_requested(&options, &received);
            }
            check_metadata = ok;
            last_check = Instant::now();
            continue;
        }

        match events.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                if options.verbose {
                    eprintln!("  event {:?} {:?}", event.kind, event.paths);
                }
                if file_events {
                    new_folders.extend(watch_new_folders(watcher.as_mut(), &event, &set));
                }
                if let Some(paths) = relevant_paths(&event, &options.magento_root) {
                    pending.extend(paths);
                    last_event = Instant::now();
                }
            }
            Ok(Err(e)) => eprintln!("{} warning: file watch error: {e}", clock()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("error: the file watcher stopped");
                std::process::exit(1);
            }
        }

        if check_metadata && pending.is_empty() && config_cache.retry_due() {
            config_cache.run_if_requested(&options, &received);
        }

        if check_metadata && pending.is_empty() && last_check.elapsed() >= METADATA_CHECK {
            last_check = Instant::now();
            if !global.exists() {
                eprintln!("{} generated/metadata removed outside the watcher", clock());
                check_metadata = false;
                compile_now = Some("generated/metadata removed".into());
            }
        }
    }
}

fn start_watcher(
    options: &Options,
    sender: mpsc::Sender<notify::Result<Event>>,
    set: &WatchSet,
) -> (Box<dyn Watcher + Send>, String) {
    if let Some(interval) = options.poll {
        return (poll_watcher(sender, set, interval), polling(interval));
    }
    let mut watcher: Box<dyn Watcher + Send> = match notify::recommended_watcher(sender.clone()) {
        Ok(watcher) => Box::new(watcher),
        Err(e) => {
            eprintln!("warning: file events unavailable ({e}); polling instead");
            return (
                poll_watcher(sender, set, POLL_DEFAULT),
                polling(POLL_DEFAULT),
            );
        }
    };
    match add_watches(watcher.as_mut(), set) {
        Ok(()) => (watcher, "file events".into()),
        Err(e) => {
            if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) {
                eprintln!(
                    "warning: inotify watch limit reached; polling instead. Raise the limit with: sysctl fs.inotify.max_user_watches=524288"
                );
            } else {
                eprintln!("warning: could not watch with file events ({e}); polling instead");
            }
            drop(watcher);
            (
                poll_watcher(sender, set, POLL_DEFAULT),
                polling(POLL_DEFAULT),
            )
        }
    }
}

fn polling(interval: Duration) -> String {
    format!("polling every {} ms", interval.as_millis())
}

fn poll_watcher(
    sender: mpsc::Sender<notify::Result<Event>>,
    set: &WatchSet,
    interval: Duration,
) -> Box<dyn Watcher + Send> {
    // Modification times can be too coarse for two saves within a second; contents are not.
    let config = notify::Config::default()
        .with_poll_interval(interval)
        .with_compare_contents(true);
    let mut watcher: Box<dyn Watcher + Send> = match notify::PollWatcher::new(sender, config) {
        Ok(watcher) => Box::new(watcher),
        Err(e) => {
            eprintln!("error: could not start polling: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = add_watches(watcher.as_mut(), set) {
        eprintln!("error: could not watch the store: {e}");
        std::process::exit(1);
    }
    watcher
}

fn add_watches(watcher: &mut (dyn Watcher + Send), set: &WatchSet) -> notify::Result<()> {
    for folder in &set.recursive {
        watcher.watch(folder, RecursiveMode::Recursive)?;
    }
    for filtered in &set.filtered {
        watcher.watch(&filtered.folder, RecursiveMode::NonRecursive)?;
    }
    Ok(())
}

fn replace_watches(watcher: &mut (dyn Watcher + Send), old: &WatchSet, new: &WatchSet) {
    for folder in old
        .recursive
        .iter()
        .chain(old.filtered.iter().map(|filtered| &filtered.folder))
    {
        let _ = watcher.unwatch(folder);
    }
    if let Err(e) = add_watches(watcher, new) {
        eprintln!("{} warning: could not watch every folder: {e}", clock());
    }
}

/// Folders created in quick succession (a module copied in, a `git checkout`) can appear between
/// the listing of a new parent folder and the start of its watch, and then stay unwatched. Each
/// new folder is watched again, recursively, at once and once more right before the next
/// compile, when the copy is complete. Returns the new folders for that second pass.
fn watch_new_folders(
    watcher: &mut (dyn Watcher + Send),
    event: &Event,
    set: &WatchSet,
) -> Vec<PathBuf> {
    if !matches!(event.kind, EventKind::Create(_)) {
        return Vec::new();
    }
    let mut created = Vec::new();
    for path in &event.paths {
        if path.is_dir() && set.recursive.iter().any(|root| path.starts_with(root)) {
            let _ = watcher.watch(path, RecursiveMode::Recursive);
            created.push(path.clone());
        }
    }
    created
}

/// The paths of `event` that call for a compile, or `None`.
fn relevant_paths(event: &Event, magento_root: &Path) -> Option<Vec<PathBuf>> {
    let changes_content = matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_) | ModifyKind::Any)
            // The poll watcher reports a write as a new modification time.
            | EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime))
            | EventKind::Access(AccessKind::Close(AccessMode::Write))
            | EventKind::Any
    );
    if !changes_content {
        return None;
    }
    let names_folder = matches!(
        event.kind,
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder)
    );
    let paths: Vec<PathBuf> = event
        .paths
        .iter()
        .filter(|path| watch_set::is_relevant(path, names_folder || path.is_dir(), magento_root))
        .cloned()
        .collect();
    (!paths.is_empty()).then_some(paths)
}

fn describe(triggers: &[PathBuf], options: &Options) -> String {
    let mut unique: Vec<&PathBuf> = triggers.iter().collect();
    unique.sort();
    unique.dedup();
    let Some(first) = unique.first() else {
        return String::new();
    };
    let root = options
        .magento_root
        .canonicalize()
        .unwrap_or_else(|_| options.magento_root.clone());
    let shown = first
        .strip_prefix(&root)
        .unwrap_or(first)
        .display()
        .to_string();
    match unique.len() {
        1 => shown,
        n => format!("{shown} +{} more", n - 1),
    }
}

struct Outcome {
    status: Option<ExitStatus>,
    stdout: String,
    stderr: String,
    killed: bool,
}

/// Runs one compile child and prints its status block. True when the compile passed.
fn compile(options: &Options, child_args: &[OsString], received: &Received, trigger: &str) -> bool {
    let started = Instant::now();
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("fast-di-compile"));
    let mut command = Command::new(exe);
    command
        .args(child_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            let removal = remove_metadata(options);
            eprintln!("{} ✗ could not start the compile: {e}", clock());
            removal.report();
            return false;
        }
    };
    let pid = child.id();
    let mut stdout = capture(child.stdout.take());
    let mut stderr = capture(child.stderr.take());

    let mut stop_sent: Option<(Instant, usize)> = None;
    let mut killed = false;
    let status = loop {
        stdout.pump();
        stderr.pump();
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        let count = received.count();
        if count > 0 {
            match stop_sent {
                None => {
                    send(pid, Stop::Terminate);
                    stop_sent = Some((Instant::now(), count));
                }
                Some((sent, seen))
                    if (count > seen && sent.elapsed() >= SAME_STOP)
                        || sent.elapsed() >= STOP_GRACE =>
                {
                    send(pid, Stop::KillGroup);
                    killed = true;
                    break child.wait().ok();
                }
                Some(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // PHP workers that are still busy belong to the child's process group.
    send(pid, Stop::KillGroup);
    let outcome = Outcome {
        status,
        stdout: stdout.finish(),
        stderr: stderr.finish(),
        killed,
    };
    if outcome.killed {
        remove_leftovers(pid);
    }
    let summary = summary_of(&outcome);
    let ok = succeeded(&outcome, summary.as_ref());
    // Before the status line, so the line always describes what is on disk.
    let removal = (!ok).then(|| remove_metadata(options));
    report(&outcome, summary.as_ref(), ok, started, trigger);
    if let Some(removal) = removal {
        removal.report();
    }
    ok
}

/// What a failed compile left the store running on.
enum Removal {
    /// No `global.php`: Magento builds its object manager from the configuration at runtime.
    Runtime,
    /// `global.php` is gone, other metadata files are left. Magento reads none of them without it.
    RuntimeWithLeftovers(std::io::Error),
    /// `global.php` is still there, so Magento keeps using the metadata of an earlier compile.
    Stale(PathBuf, std::io::Error),
}

impl Removal {
    fn report(&self) {
        match self {
            Self::Runtime => eprintln!("{RUNTIME_MODE}"),
            Self::RuntimeWithLeftovers(e) => {
                eprintln!("{RUNTIME_MODE}");
                eprintln!("warning: some files in generated/metadata could not be removed: {e}");
            }
            Self::Stale(path, e) => {
                eprintln!("error: could not remove {}: {e}", path.display());
                eprintln!(
                    "store still runs on the metadata of an earlier compile; \
                     fix the permissions of generated/metadata or remove it"
                );
            }
        }
    }
}

const RUNTIME_MODE: &str = "store runs without compiled metadata until the next successful compile";

/// After a failed compile the store must never run on the metadata of an earlier compile. A
/// compile that stops early (an incompatible module, for example) leaves its output alone, as a
/// one-off compile should, so the watcher removes the metadata itself.
///
/// `EnvironmentFactory` picks the compiled object manager by the existence of `global.php`, so
/// that file goes first and decides what the store runs on.
fn remove_metadata(options: &Options) -> Removal {
    let metadata = options.generated_root.join("metadata");
    let global = metadata.join("global.php");
    match std::fs::remove_file(&global) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Removal::Stale(global, e),
        _ => {}
    }
    match std::fs::remove_dir_all(&metadata) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Removal::RuntimeWithLeftovers(e),
        _ => Removal::Runtime,
    }
}

fn summary_of(outcome: &Outcome) -> Option<serde_json::Value> {
    outcome
        .stdout
        .lines()
        .rev()
        .find(|line| line.starts_with('{'))
        .and_then(|line| serde_json::from_str(line).ok())
}

fn succeeded(outcome: &Outcome, summary: Option<&serde_json::Value>) -> bool {
    summary
        .and_then(|summary| summary["ok"].as_bool())
        .unwrap_or(false)
        && outcome.status.is_some_and(|status| status.success())
}

fn report(
    outcome: &Outcome,
    summary: Option<&serde_json::Value>,
    ok: bool,
    started: Instant,
    trigger: &str,
) {
    let seconds = summary
        .and_then(|summary| summary["duration_ms"].as_u64())
        .map(|ms| ms as f64 / 1000.0)
        .unwrap_or_else(|| started.elapsed().as_secs_f64());
    let written = summary
        .and_then(|summary| summary["files_written"].as_u64())
        .unwrap_or(0);
    let suffix = if trigger.is_empty() {
        String::new()
    } else {
        format!(" — {trigger}")
    };
    if ok {
        let files = if written == 1 { "file" } else { "files" };
        eprintln!(
            "{} ✓ compiled in {seconds:.2}s, {written} {files} written{suffix}",
            clock()
        );
    } else {
        eprintln!("{} ✗ failed in {seconds:.2}s{suffix}", clock());
    }
    let log = outcome.stderr.trim_end();
    if !log.is_empty() {
        eprintln!("{log}");
    }
    if !ok {
        if outcome.killed {
            eprintln!(
                "error: the compile did not stop within {}s and was killed",
                STOP_GRACE.as_secs()
            );
        } else if summary.is_none() {
            match outcome.status.and_then(|status| status.code()) {
                Some(code) => eprintln!("error: the compile ended with exit code {code}"),
                None => eprintln!("error: the compile ended by a signal"),
            }
        }
    }
}

/// A child's pipe, read without blocking by the loop that waits for the child. The child never
/// blocks on a full pipe, and a process the child started can hold the pipe open without holding
/// a thread or a file descriptor of the watcher.
struct Capture {
    pipe: Option<Box<dyn Read>>,
    bytes: Vec<u8>,
}

#[cfg(unix)]
fn capture<R: Read + std::os::fd::AsRawFd + 'static>(pipe: Option<R>) -> Capture {
    let pipe = pipe.map(|pipe| {
        let fd = pipe.as_raw_fd();
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        Box::new(pipe) as Box<dyn Read>
    });
    Capture {
        pipe,
        bytes: Vec::new(),
    }
}

#[cfg(not(unix))]
fn capture<R: Read + 'static>(pipe: Option<R>) -> Capture {
    Capture {
        pipe: pipe.map(|pipe| Box::new(pipe) as Box<dyn Read>),
        bytes: Vec::new(),
    }
}

impl Capture {
    /// Takes what the pipe holds now. True once the pipe has ended, which also closes it.
    fn pump(&mut self) -> bool {
        if cfg!(not(unix)) {
            return self.pipe.is_none();
        }
        let Some(pipe) = self.pipe.as_mut() else {
            return true;
        };
        let mut chunk = [0u8; 8192];
        let ended = loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break true,
                Ok(n) => self.bytes.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break false,
                Err(_) => break true,
            }
        };
        if ended {
            self.pipe = None;
        }
        ended
    }

    /// The text of the pipe, called once the child has ended. A process the child started can
    /// keep the pipe open long after that, so the wait for the end of the pipe is short; then the
    /// pipe is closed and what arrived until then is the result.
    fn finish(mut self) -> String {
        #[cfg(not(unix))]
        if let Some(mut pipe) = self.pipe.take() {
            let _ = pipe.read_to_end(&mut self.bytes);
        }
        let until = Instant::now() + PIPE_DRAIN;
        while !self.pump() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

enum Stop {
    /// SIGTERM to the compile only, which then cleans up after itself.
    Terminate,
    /// SIGKILL to the compile and every PHP worker in its process group.
    KillGroup,
}

#[cfg(unix)]
fn send(pid: u32, stop: Stop) {
    let pid = pid as libc::pid_t;
    unsafe {
        match stop {
            Stop::Terminate => {
                libc::kill(pid, libc::SIGTERM);
            }
            Stop::KillGroup => {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(not(unix))]
fn send(_pid: u32, _stop: Stop) {}

/// The worker files a killed compile could not remove itself, as `stop_compile()` would have.
/// `remove_metadata()` takes care of the metadata after every failed compile.
fn remove_leftovers(pid: u32) {
    let temp = std::env::temp_dir();
    let _ = std::fs::remove_file(temp.join(format!("fast-di-worker-{pid}.php")));
    let _ = std::fs::remove_file(temp.join(format!("fast-di-classes-{pid}.txt")));
}

/// The module list lives in caches too: routes in `config`, and in the compiled environment the
/// controller list of `App\Router\ActionList` in `compiled_config`. A change of
/// `app/etc/config.php` needs both cleaned after a passing compile, or a newly enabled module's
/// pages answer 404. A failed clean stays requested and is tried again with growing pauses.
struct ConfigCacheClean {
    requested: bool,
    retry_at: Option<Instant>,
    pause: Duration,
}

impl Default for ConfigCacheClean {
    fn default() -> Self {
        Self {
            requested: false,
            retry_at: None,
            pause: CLEAN_RETRY_FIRST,
        }
    }
}

impl ConfigCacheClean {
    fn request(&mut self) {
        self.requested = true;
        self.retry_at = None;
        self.pause = CLEAN_RETRY_FIRST;
    }

    fn retry_due(&self) -> bool {
        self.requested && self.retry_at.is_some_and(|at| Instant::now() >= at)
    }

    fn run_if_requested(&mut self, options: &Options, received: &Received) {
        if !self.requested {
            return;
        }
        match clean_config(options, received) {
            Clean::Done => *self = Self::default(),
            Clean::Failed => {
                eprintln!(
                    "{}   cache:clean config compiled_config will be tried again in {}s",
                    clock(),
                    self.pause.as_secs()
                );
                self.retry_at = Some(Instant::now() + self.pause);
                self.pause = (self.pause * 2).min(CLEAN_RETRY_MAX);
            }
            Clean::Interrupted => {}
        }
    }
}

enum Clean {
    Done,
    Failed,
    Interrupted,
}

/// Runs `bin/magento cache:clean config compiled_config` in its own process group, so a stop
/// request or the timeout can end it together with anything it started.
fn clean_config(options: &Options, received: &Received) -> Clean {
    let magento = options.magento_root.join("bin/magento");
    if !magento.is_file() {
        return Clean::Done;
    }
    let mut command = Command::new(&options.php);
    command
        .arg(&magento)
        .args(["cache:clean", "config", "compiled_config", "--quiet"])
        .current_dir(&options.magento_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            eprintln!(
                "{} warning: could not run cache:clean config compiled_config: {e}",
                clock()
            );
            return Clean::Failed;
        }
    };
    let pid = child.id();
    let mut stderr = capture(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        stderr.pump();
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if received.count() > 0 {
            send(pid, Stop::KillGroup);
            let _ = child.wait();
            return Clean::Interrupted;
        }
        if started.elapsed() >= CLEAN_TIMEOUT {
            send(pid, Stop::KillGroup);
            let _ = child.wait();
            eprintln!(
                "{} warning: cache:clean config compiled_config did not finish within {}s",
                clock(),
                CLEAN_TIMEOUT.as_secs()
            );
            return Clean::Failed;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Whatever the cache command left running in its group ends with it, as after a compile.
    send(pid, Stop::KillGroup);
    let stderr = stderr.finish();
    if status.is_some_and(|status| status.success()) {
        eprintln!(
            "{}   cache:clean config compiled_config (module list changed)",
            clock()
        );
        return Clean::Done;
    }
    eprintln!(
        "{} warning: cache:clean config compiled_config failed: {}",
        clock(),
        stderr.trim()
    );
    Clean::Failed
}

#[cfg(unix)]
fn clock() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&now, &mut parts);
    }
    format!(
        "{:02}:{:02}:{:02}",
        parts.tm_hour, parts.tm_min, parts.tm_sec
    )
}

#[cfg(not(unix))]
fn clock() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(arguments: &[OsString]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_child_gets_the_compile_arguments_and_the_output_flags() {
        let given = [
            "--magento-root",
            "/srv",
            "--watch",
            "--watch-exit-with-parent",
            "-j",
            "4",
            "--quiet",
            "--summary-json",
        ]
        .map(OsString::from);
        assert_eq!(
            strings(&child_arguments(given, false)),
            [
                "--magento-root",
                "/srv",
                "-j",
                "4",
                "--quiet",
                "--summary-json"
            ]
        );
        let polled = ["--watch-poll=250", "--magento-root", "/srv", "-v"].map(OsString::from);
        assert_eq!(
            strings(&child_arguments(polled, true)),
            ["--magento-root", "/srv", "-v", "--summary-json"]
        );
    }

    #[test]
    fn content_changes_count_and_metadata_changes_do_not() {
        let root = tempfile::TempDir::new().unwrap();
        let file = root.path().join("app/code/Acme/On/etc/di.xml");
        let event = |kind| Event::new(kind).add_path(file.clone());
        assert!(relevant_paths(&event(EventKind::Create(CreateKind::File)), root.path()).is_some());
        assert!(relevant_paths(
            &event(EventKind::Modify(ModifyKind::Name(
                notify::event::RenameMode::To
            ))),
            root.path()
        )
        .is_some());
        assert!(relevant_paths(
            &event(EventKind::Modify(ModifyKind::Metadata(
                notify::event::MetadataKind::Ownership
            ))),
            root.path()
        )
        .is_none());
        assert!(relevant_paths(
            &event(EventKind::Modify(ModifyKind::Metadata(
                MetadataKind::WriteTime
            ))),
            root.path()
        )
        .is_some());
        assert!(relevant_paths(
            &event(EventKind::Access(AccessKind::Open(AccessMode::Read))),
            root.path()
        )
        .is_none());
    }
}

//! The end of a compile: one machine-readable summary line for a caller such as watch mode, the
//! cleanup a stopped compile needs, and the handling of SIGINT and SIGTERM.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static STARTED: OnceLock<Instant> = OnceLock::new();
static SUMMARY_JSON: AtomicBool = AtomicBool::new(false);
static SUMMARY_PRINTED: AtomicBool = AtomicBool::new(false);
static CLEANUP: OnceLock<Vec<CleanupStep>> = OnceLock::new();

enum CleanupStep {
    RemoveFolder(PathBuf),
    RemoveFile(PathBuf),
}

/// Starts the clock and decides whether the summary line is printed.
pub fn start(summary_json: bool) {
    STARTED.get_or_init(Instant::now);
    SUMMARY_JSON.store(summary_json, Ordering::SeqCst);
}

/// What a stopped compile removes: the metadata folder (so the store never runs on a partial
/// set), the PHP worker script and the class list.
pub fn on_stop_remove(metadata_root: Option<PathBuf>, files: Vec<PathBuf>) {
    let mut steps: Vec<CleanupStep> = metadata_root
        .into_iter()
        .map(CleanupStep::RemoveFolder)
        .collect();
    steps.extend(files.into_iter().map(CleanupStep::RemoveFile));
    let _ = CLEANUP.set(steps);
}

/// Stops every later write, then removes what [`on_stop_remove`] named. A write that was already
/// past its checks can still land, so the folder removal is tried twice.
pub fn clean_up() {
    code_generator::stop_writes();
    for step in CLEANUP.get().map(Vec::as_slice).unwrap_or_default() {
        match step {
            CleanupStep::RemoveFolder(folder) => {
                if std::fs::remove_dir_all(folder).is_err() && folder.exists() {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let _ = std::fs::remove_dir_all(folder);
                }
            }
            CleanupStep::RemoveFile(file) => {
                let _ = std::fs::remove_file(file);
            }
        }
    }
}

/// Prints the summary line on stdout once, when `--summary-json` asked for it.
pub fn print_summary(error: Option<&str>) {
    if !SUMMARY_JSON.load(Ordering::SeqCst) || SUMMARY_PRINTED.swap(true, Ordering::SeqCst) {
        return;
    }
    println!("{}", summary_line(error));
}

fn summary_line(error: Option<&str>) -> String {
    let duration_ms = STARTED
        .get()
        .map(|started| started.elapsed().as_millis())
        .unwrap_or_default();
    let (written, unchanged) = code_generator::write_counts();
    let mut line = serde_json::Map::new();
    line.insert("ok".into(), error.is_none().into());
    line.insert("duration_ms".into(), (duration_ms as u64).into());
    line.insert("files_written".into(), written.into());
    line.insert("files_unchanged".into(), unchanged.into());
    if let Some(error) = error {
        line.insert("error".into(), error.into());
    }
    serde_json::Value::Object(line).to_string()
}

/// Ends the process with `code`, printing the summary first.
pub fn exit(code: i32, error: Option<&str>) -> ! {
    print_summary(error);
    std::process::exit(code);
}

/// On SIGINT or SIGTERM: stop writes, clean up as a failed compile does, print the summary with
/// `"interrupted"`, and exit with 128 + the signal number.
#[cfg(unix)]
pub fn stop_on_signals() {
    use signal_hook::consts::{SIGINT, SIGTERM};
    // Test hook: a compile that does not stop on SIGTERM, so the watcher's kill path can be tested.
    if std::env::var_os("FAST_DI_COMPILE_TEST_IGNORE_SIGTERM").is_some() {
        let ignored = std::sync::Arc::new(AtomicBool::new(false));
        let _ = signal_hook::flag::register(SIGTERM, ignored);
        return;
    }
    let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGINT, SIGTERM]) else {
        log::warn!("Could not install the SIGINT/SIGTERM handler");
        return;
    };
    std::thread::spawn(move || {
        if let Some(signal) = signals.forever().next() {
            eprintln!("error: interrupted by signal {signal}");
            clean_up();
            exit(128 + signal, Some("interrupted"));
        }
    });
}

#[cfg(not(unix))]
pub fn stop_on_signals() {}

#[cfg(test)]
mod tests {
    use super::summary_line;

    #[test]
    fn a_failed_compile_names_its_error() {
        let line: serde_json::Value = serde_json::from_str(&summary_line(Some("broken"))).unwrap();
        assert_eq!(line["ok"], false);
        assert_eq!(line["error"], "broken");
        assert!(line["files_written"].is_u64());
        assert!(line["duration_ms"].is_u64());
    }

    #[test]
    fn a_passing_compile_has_no_error() {
        let line: serde_json::Value = serde_json::from_str(&summary_line(None)).unwrap();
        assert_eq!(line["ok"], true);
        assert!(line.get("error").is_none());
    }
}

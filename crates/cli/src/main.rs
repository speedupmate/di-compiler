//! TKT-024/025/026: fast-di-compile CLI binary.
//!
//! Phases:
//!   1. Walk all PHP files (rayon parallel — TKT-025)
//!   2. Extract ClassInfo (3-tier: lexer → tree-sitter → PHP shell)
//!   3. Parse + merge all di.xml files
//!   4. Detect interceptors / factories / proxies
//!   5. Resolve constructor arguments
//!   6. Generate PHP code files (interceptors, factories, proxies)
//!   7. Generate metadata files (area configs, interception.php)
//!   8. Incremental writes (skip unchanged — TKT-022/026)

use std::collections::{BTreeMap, BTreeSet};
use rustc_hash::{FxHashMap, FxHashSet};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use quick_xml::events::Event;
use quick_xml::Reader;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use code_generator::{
    extension_path, factory_path, generate_app_action_list_php,
    generate_area_config_with_overrides, generate_extension, generate_extension_interface,
    compile_plugin_list, generate_factory, generate_interceptor, generate_proxy,
    generate_proxy_deferred, generate_search_results, interceptor_path,
    proxy_deferred_path, proxy_path, search_results_path, serialize_interception_php,
    serialize_plugin_list_php, write_if_changed,
    ExtensionAttributeSpec, ExtensionSpec, AREAS,
};
use di_resolver::{
    detect_factories_from_configs, detect_interceptors, detect_proxies_from_configs_with_existing,
    resolve_all_arguments, resolve_all_arguments_for_named_types, FactorySpec,
};
use di_xml_reader::{
    apply_module_config_on_primary, find_all_di_xml_files, find_di_xml_files,
    find_di_xml_files_for_area, merge_configs, merge_into, parse_di_xml, Argument, DiConfig,
    Plugin,
};
use php_extractor::{
    extract_file, extract_string_constants,
    types::{
        ClassInfo, ClassKind, Constructor, ConstructorParam, ExtractResult, MethodParam,
        MethodSignature,
    },
    walker::{read_module_paths, walk_php_files},
};

#[derive(Parser, Debug)]
#[command(
    name = "fast-di-compile",
    about = "Fast Rust replacement for bin/magento setup:di:compile"
)]
struct Args {
    /// Magento root directory
    #[arg(long, default_value = ".")]
    magento_root: PathBuf,

    /// Number of parallel jobs (default: number of CPUs)
    #[arg(long, short = 'j')]
    jobs: Option<usize>,

    /// Path to PHP binary for Tier 3 fallback
    #[arg(long, default_value = "php")]
    fallback_php: String,

    /// Validate output against PHP ground truth
    #[arg(long)]
    validate: bool,

    /// PHP ground-truth generated dir (used with --validate)
    #[arg(long)]
    php_generated: Option<PathBuf>,

    /// Output directory (default: <magento-root>/generated)
    #[arg(long)]
    output: Option<PathBuf>,

    /// Enable incremental compilation (skip unchanged files)
    #[arg(long)]
    incremental: bool,

    /// Dry run — do not write output files
    #[arg(long)]
    dry_run: bool,

    /// Verbose logging
    #[arg(long, short = 'v')]
    verbose: bool,

    /// Compare output against archive baseline (_code/_metadata) after generation
    #[arg(long)]
    compare_archive: bool,

    /// Archive root containing _code and _metadata (default: <magento-root>/generated)
    #[arg(long)]
    archive_root: Option<PathBuf>,

    /// Where to write archive diff reports (default: <output>/diff)
    #[arg(long)]
    compare_report_dir: Option<PathBuf>,

    /// Exit with code 1 when archive comparison has differences
    #[arg(long)]
    compare_fail_on_diff: bool,
}

// ---------------------------------------------------------------------------
// Incremental cache (TKT-026)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default)]
struct IncrementalCache {
    /// Map of absolute path → blake3 hex hash of that file at last compile
    files: FxHashMap<String, String>,
}

impl IncrementalCache {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    fn hash_of(path: &Path) -> Option<String> {
        let data = std::fs::read(path).ok()?;
        Some(blake3::hash(&data).to_hex().to_string())
    }

    fn record(&mut self, path: &Path) {
        if let Some(hash) = Self::hash_of(path) {
            self.files.insert(path.to_string_lossy().to_string(), hash);
        }
    }
}

// ---------------------------------------------------------------------------
// Persistent PHP worker pool
// ---------------------------------------------------------------------------

/// PHP script run as a long-lived worker: reads "cmd:FQCN\n" from stdin,
/// writes one JSON line to stdout per request. Autoload is loaded once.
const WORKER_PHP: &str = r#"<?php
declare(strict_types=1);
ini_set('display_errors', '0');
ini_set('display_startup_errors', '0');
$root = $argv[1] ?? '';
if ($root === '' || !file_exists($root . '/vendor/autoload.php')) {
    fwrite(STDERR, "php-worker: missing magento root\n");
    exit(1);
}
@require $root . '/vendor/autoload.php';
function tstr($t): ?string {
    if ($t === null) return null;
    if ($t instanceof ReflectionNamedType) {
        $name = $t->getName();
        if ($t->allowsNull() && $name !== 'mixed' && $name !== 'null') return '?' . $name;
        return $name;
    }
    if ($t instanceof ReflectionUnionType) {
        $parts = []; foreach ($t->getTypes() as $p) $parts[] = $p->getName();
        return implode('|', $parts);
    }
    if ($t instanceof ReflectionIntersectionType) {
        $parts = []; foreach ($t->getTypes() as $p) $parts[] = $p->getName();
        return implode('&', $parts);
    }
    return null;
}
function reflect_methods(string $class): ?array {
    if (!class_exists($class) && !interface_exists($class) && !trait_exists($class)) return null;
    $ref = new ReflectionClass($class);
    $rows = [];
    foreach ($ref->getMethods(ReflectionMethod::IS_PUBLIC) as $m) {
        if ($m->isConstructor() || $m->isFinal() || $m->isStatic() || $m->isDestructor()) continue;
        $name = $m->getName();
        if (in_array($name, ['__sleep','__wakeup','__clone'], true)) continue;
        $params = [];
        foreach ($m->getParameters() as $p) {
            $dv = null; $hd = $p->isDefaultValueAvailable() && !$p->isVariadic();
            if ($hd) { $raw = $p->getDefaultValue(); $dv = is_array($raw) ? '__json__:'.json_encode($raw) : var_export($raw, true); }
            $params[] = ['name'=>$p->getName(),'type_hint'=>tstr($p->getType()),
                'has_default'=>$hd,'default_value'=>$dv,
                'is_variadic'=>$p->isVariadic(),'is_by_ref'=>$p->isPassedByReference()];
        }
        $rows[] = ['name'=>$name,'params'=>$params,
            'return_type'=>tstr($m->getReturnType()),'returns_reference'=>$m->returnsReference()];
    }
    return $rows;
}
function reflect_kind(string $class): ?string {
    if (!class_exists($class) && !interface_exists($class) && !trait_exists($class)) return null;
    $ref = new ReflectionClass($class);
    if ($ref->isInterface()) return 'interface';
    if ($ref->isTrait()) return 'trait';
    return 'class';
}
function reflect_const(string $expr) {
    $expr = ltrim($expr, '\\');
    if ($expr === '' || !defined($expr)) return null;
    return constant($expr);
}
function reflect_ctor(string $class): ?array {
    if (!class_exists($class) && !interface_exists($class) && !trait_exists($class)) return null;
    $ref = new ReflectionClass($class);
    $ctor = $ref->getConstructor();
    if ($ctor === null) return null;
    $params = [];
    foreach ($ctor->getParameters() as $p) {
        $dv = null; $hd = $p->isDefaultValueAvailable() && !$p->isVariadic();
        if ($hd) { $raw = $p->getDefaultValue(); $dv = is_array($raw) ? '__json__:'.json_encode($raw) : var_export($raw, true); }
        $params[] = ['name'=>$p->getName(),'type_hint'=>tstr($p->getType()),
            'has_default'=>$hd,'default_value'=>$dv,'is_variadic'=>$p->isVariadic()];
    }
    return $params;
}
$stdin = fopen('php://stdin', 'r');
while (($line = fgets($stdin)) !== false) {
    $line = rtrim($line, "\r\n");
    if ($line === 'exit') break;
    $colon = strpos($line, ':');
    if ($colon === false) { fwrite(STDOUT, "null\n"); fflush(STDOUT); continue; }
    $cmd = substr($line, 0, $colon);
    $class = ltrim(substr($line, $colon + 1), '\\');
    try {
        $result = match($cmd) {
            'methods' => reflect_methods($class),
            'kind'    => reflect_kind($class),
            'const'   => reflect_const($class),
            'ctor'    => reflect_ctor($class),
            default   => null,
        };
        fwrite(STDOUT, json_encode($result) . "\n");
    } catch (\Throwable $e) {
        fwrite(STDOUT, "null\n");
    }
    fflush(STDOUT);
}
"#;

struct PhpWorker {
    child: std::process::Child,
    stdin: Option<BufWriter<std::process::ChildStdin>>,
    stdout: BufReader<std::process::ChildStdout>,
}

impl PhpWorker {
    fn spawn(php_bin: &str, magento_root: &Path, script_path: &Path) -> Option<Self> {
        let mut child = Command::new(php_bin)
            .arg("-d")
            .arg("display_errors=0")
            .arg("-d")
            .arg("display_startup_errors=0")
            .arg(script_path)
            .arg(magento_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = BufWriter::new(child.stdin.take()?);
        let stdout = BufReader::new(child.stdout.take()?);
        Some(PhpWorker {
            child,
            stdin: Some(stdin),
            stdout,
        })
    }

    /// Send a request line and read back one JSON line. Returns None if the
    /// worker pipe is broken (worker died).
    fn request(&mut self, line: &str) -> Option<String> {
        let w = self.stdin.as_mut()?;
        w.write_all(line.as_bytes()).ok()?;
        w.write_all(b"\n").ok()?;
        w.flush().ok()?;
        let mut resp = String::new();
        self.stdout.read_line(&mut resp).ok()?;
        if resp.is_empty() {
            return None;
        } // EOF — worker died
        Some(resp.trim_end_matches('\n').to_string())
    }
}

impl Drop for PhpWorker {
    fn drop(&mut self) {
        drop(self.stdin.take()); // closing stdin signals PHP loop to exit
        let _ = self.child.wait();
    }
}

struct PhpWorkerPool {
    workers: Mutex<Vec<PhpWorker>>,
    php_bin: String,
    magento_root: PathBuf,
    script_path: PathBuf,
}

impl PhpWorkerPool {
    fn new(php_bin: String, magento_root: PathBuf, script_path: PathBuf) -> Self {
        PhpWorkerPool {
            workers: Mutex::new(Vec::new()),
            php_bin,
            magento_root,
            script_path,
        }
    }

    fn checkout(&self) -> Option<PhpWorker> {
        self.workers
            .lock()
            .unwrap()
            .pop()
            .or_else(|| PhpWorker::spawn(&self.php_bin, &self.magento_root, &self.script_path))
    }

    fn checkin(&self, w: PhpWorker) {
        self.workers.lock().unwrap().push(w);
    }

    /// Run a single request, automatically retrying once with a fresh worker
    /// if the checked-out worker has died.
    fn request(&self, line: &str) -> Option<String> {
        for _ in 0..2 {
            let mut w = self.checkout()?;
            match w.request(line) {
                Some(resp) => {
                    self.checkin(w);
                    return Some(resp);
                }
                None => { /* worker died — drop it, loop spawns a fresh one */ }
            }
        }
        None
    }
}

static PHP_WORKER_POOL: OnceLock<PhpWorkerPool> = OnceLock::new();

fn init_php_worker_pool(php_bin: &str, magento_root: &Path) -> PathBuf {
    let script_path =
        std::env::temp_dir().join(format!("fast-di-worker-{}.php", std::process::id()));
    std::fs::write(&script_path, WORKER_PHP)
        .expect("failed to write PHP worker script to temp dir");
    PHP_WORKER_POOL.get_or_init(|| {
        PhpWorkerPool::new(
            php_bin.to_string(),
            magento_root.to_path_buf(),
            script_path.clone(),
        )
    });
    script_path
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = Args::parse();

    env_logger::Builder::new()
        .filter_level(if args.verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .init();

    // Wire --fallback-php to the env var that Tier 3 reads.
    if args.fallback_php != "php" {
        std::env::set_var("FAST_DI_PHP", &args.fallback_php);
    }

    // Validate requires --php-generated
    if args.validate && args.php_generated.is_none() {
        eprintln!("error: --validate requires --php-generated <dir>");
        std::process::exit(2);
    }

    // Configure rayon thread pool
    if let Some(jobs) = args.jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build_global()
            .unwrap_or_default();
    }

    let magento_root = args
        .magento_root
        .canonicalize()
        .unwrap_or(args.magento_root.clone());
    let generated_root = args
        .output
        .clone()
        .unwrap_or_else(|| magento_root.join("generated"));
    let code_root = generated_root.join("code");
    let metadata_root = generated_root.join("metadata");

    // Initialise the persistent PHP worker pool (one autoload per worker process).
    let worker_script_path = init_php_worker_pool(&args.fallback_php, &magento_root);

    log::info!(
        "fast-di-compile starting\n  magento_root: {}\n  output:       {}",
        magento_root.display(),
        generated_root.display()
    );
    let total_started = Instant::now();

    // Incremental cache
    let cache_path = generated_root.join(".di-compiler-cache.json");
    let mut cache = if args.incremental {
        IncrementalCache::load(&cache_path)
    } else {
        IncrementalCache::default()
    };

    // -----------------------------------------------------------------------
    // Phase 1 + 2: Walk PHP files + extract ClassInfo (parallel)
    // -----------------------------------------------------------------------
    let phase_1_2_started = Instant::now();
    let module_paths = read_module_paths(&magento_root);
    let php_files = walk_php_files(&module_paths);
    log::info!("Found {} PHP files", php_files.len());

    let pb = progress_bar(php_files.len() as u64, "Extracting PHP classes");

    let fallback_count = std::sync::atomic::AtomicUsize::new(0);
    let failure_count = std::sync::atomic::AtomicUsize::new(0);

    // (cache_ref is unused since di.xml parse-skip was removed as a correctness fix)
    let _cache_ref = &cache;

    let class_map: FxHashMap<String, ClassInfo> = php_files
        .par_iter()
        .filter_map(|path| {
            let result = extract_file(path);
            pb.inc(1);
            match result {
                ExtractResult::Ok(info) => Some((info.fqcn.clone(), info)),
                ExtractResult::NoClass => None,
                ExtractResult::PhpFallbackFailed(e) => {
                    fallback_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    log::warn!("Fallback failed for {}: {}", path.display(), e);
                    None
                }
                ExtractResult::LexError(e) => {
                    failure_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    log::warn!("Lex error for {}: {e}", path.display());
                    None
                }
                ExtractResult::ParseFailure(e) => {
                    failure_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    log::warn!("Parse failure for {}: {}", path.display(), e);
                    None
                }
            }
        })
        .collect();
    pb.finish_with_message("done");

    let fallbacks = fallback_count.load(std::sync::atomic::Ordering::Relaxed);
    let failures = failure_count.load(std::sync::atomic::Ordering::Relaxed);
    log::info!(
        "Extracted {} classes  ({} fallbacks, {} failures)",
        class_map.len(),
        fallbacks,
        failures
    );
    log_phase_elapsed("Phase 1+2", phase_1_2_started);

    // -----------------------------------------------------------------------
    // Phase 3a: Parse + merge global di.xml files (for per-area metadata)
    // -----------------------------------------------------------------------
    let phase_3a_started = Instant::now();
    let enabled_modules = load_module_order_from_config_php(&magento_root);
    let di_xml_files = filter_enabled_di_xml(
        find_di_xml_files(&magento_root, &enabled_modules),
        &enabled_modules,
        &magento_root,
    );
    log::info!("Found {} di.xml files (global)", di_xml_files.len());

    let pb = progress_bar(di_xml_files.len() as u64, "Parsing di.xml (global)");
    // Parse with path retained so we can split primary (app/etc/di.xml) from modules.
    // NOTE: di.xml files must always be parsed — skipping unchanged files would drop
    // them from the merge entirely, producing an incomplete di_config (correctness bug).
    let global_di_path_configs: Vec<_> = di_xml_files
        .par_iter()
        .filter_map(|path| {
            let r = parse_di_xml(path);
            pb.inc(1);
            match r {
                Ok(cfg) => Some((path.clone(), cfg)),
                Err(e) => {
                    log::warn!("di.xml parse error {}: {e}", path.display());
                    None
                }
            }
        })
        .collect();
    pb.finish_with_message("done");

    // Two-phase merge replicating PHP's Config::_mergeConfiguration behaviour:
    //   Phase 1: deep-merge all module di.xml files together (items accumulate by name)
    //   Phase 2: apply merged module result on app/etc/di.xml with shallow arg replacement
    //            (PHP uses array_replace at argument-name level, not item-level deep merge)
    let app_etc_di_path = magento_root.join("app/etc/di.xml");
    let (primary_configs, module_configs): (Vec<_>, Vec<_>) = global_di_path_configs
        .iter()
        .partition(|(p, _)| *p == app_etc_di_path);
    let primary_base = merge_configs(
        primary_configs
            .into_iter()
            .map(|(_, c)| c.clone())
            .collect(),
    );
    let module_merged = merge_configs(module_configs.into_iter().map(|(_, c)| c.clone()).collect());
    let di_config = apply_module_config_on_primary(primary_base, module_merged);

    // Flat Vec<DiConfig> (without path) for detection purposes (plugins/types, not arg values).
    let global_di_configs: Vec<DiConfig> =
        global_di_path_configs.into_iter().map(|(_, c)| c).collect();
    // HashSet for O(1) membership tests — used in Phase 3b and Phase 7 filters.
    let di_xml_files_set: FxHashSet<&PathBuf> = di_xml_files.iter().collect();
    log_phase_elapsed("Phase 3a", phase_3a_started);

    // -----------------------------------------------------------------------
    // Phase 3b: Parse + merge ALL di.xml files (all areas) for detection
    //
    // Interceptor/factory/proxy detection must consider plugins registered in
    // area-specific di.xml files (e.g. etc/adminhtml/di.xml), not just global.
    // -----------------------------------------------------------------------
    let phase_3b_started = Instant::now();
    let all_di_xml_files = filter_enabled_di_xml(
        find_all_di_xml_files(&magento_root, &enabled_modules),
        &enabled_modules,
        &magento_root,
    );
    log::info!("Found {} di.xml files (all areas)", all_di_xml_files.len());

    // Only parse files not already in the global set
    let extra_di_files: Vec<_> = all_di_xml_files
        .iter()
        .filter(|p| !di_xml_files_set.contains(p))
        .collect();

    // Parse area-specific di.xml files into a path-keyed cache so the Phase 7
    // area loop can look up already-parsed configs instead of re-parsing.
    let area_di_xml_cache: FxHashMap<std::path::PathBuf, DiConfig> = extra_di_files
        .par_iter()
        .filter_map(|path| {
            parse_di_xml(path)
                .ok()
                .map(|cfg| ((*path).clone(), cfg))
        })
        .collect();
    // Preserve original file order for deterministic scanner_di_configs merge.
    let extra_configs: Vec<DiConfig> = extra_di_files
        .iter()
        .filter_map(|path| area_di_xml_cache.get(*path).cloned())
        .collect();

    let mut scanner_di_configs = global_di_configs.clone();
    scanner_di_configs.extend(extra_configs.clone());

    let full_di_config = if extra_configs.is_empty() {
        di_config.clone()
    } else {
        let mut full = di_config.clone();
        let extra_merged = merge_configs(extra_configs);
        merge_into(&mut full, extra_merged);
        full
    };

    log::info!(
        "DI config: {} preferences, {} plugins, {} virtualTypes (global: {}/{}/{})",
        full_di_config.preferences.len(),
        full_di_config.plugins.len(),
        full_di_config.virtual_types.len(),
        di_config.preferences.len(),
        di_config.plugins.len(),
        di_config.virtual_types.len(),
    );
    log_phase_elapsed("Phase 3b", phase_3b_started);

    // -----------------------------------------------------------------------
    // Phase 3c: Collect extension-attributes metadata (for Extension* artifacts)
    // -----------------------------------------------------------------------
    let phase_3c_started = Instant::now();
    let extension_attr_files = find_extension_attributes_files(&magento_root, &module_paths);
    let extension_attr_map = parse_extension_attributes_files(&extension_attr_files);
    let extension_specs = collect_extension_specs(&class_map, &extension_attr_map);
    log_phase_elapsed("Phase 3c", phase_3c_started);

    // -----------------------------------------------------------------------
    // Phase 4: Detection (uses full_di_config = all areas merged)
    // -----------------------------------------------------------------------
    let phase_4_started = Instant::now();
    let composer_autoload = ComposerAutoloadIndex::from_magento_root(&magento_root);
    let proxy_targets = collect_proxy_targets_from_di_configs(&scanner_di_configs);
    let mut extra_existing_proxy_targets: FxHashSet<String> = FxHashSet::default();
    if let Some(index) = &composer_autoload {
        for target in proxy_targets {
            if class_map.contains_key(&target) {
                continue;
            }
            // Keep Magento namespace existence tied to scanned ClassInfo scope.
            // Composer fallback is for third-party/autoload-only targets (e.g. PSR).
            if target.starts_with("Magento\\") {
                continue;
            }
            if index.is_loadable(&target) {
                extra_existing_proxy_targets.insert(target);
            }
        }
    }

    let mut factories =
        detect_factories_from_configs(&class_map, &full_di_config, &scanner_di_configs, &di_config);
    // Drop factories for classes that Composer already knows about (they exist in
    // a non-scanned vendor package, e.g. ramsey/uuid). PHP skips these too.
    if let Some(index) = &composer_autoload {
        factories.retain(|spec| !index.is_loadable(&spec.factory_fqcn));
    }
    let proxies = detect_proxies_from_configs_with_existing(
        &class_map,
        &scanner_di_configs,
        &extra_existing_proxy_targets,
    );

    // Extension class factories are generated by Magento's PHP scanner path.
    // Include them even when no constructor/XML factory trigger references them.
    let mut factory_seen: FxHashSet<String> = factories
        .iter()
        .map(|spec| spec.factory_fqcn.clone())
        .collect();
    for ext in &extension_specs {
        let factory_fqcn = format!("{}Factory", ext.extension_class_fqcn);
        if class_map.contains_key(&factory_fqcn) || !factory_seen.insert(factory_fqcn.clone()) {
            continue;
        }
        factories.push(FactorySpec {
            target_fqcn: ext.extension_class_fqcn.clone(),
            factory_fqcn,
        });
    }

    let search_results = detect_search_results_specs(
        &class_map,
        &full_di_config,
        &factories,
        composer_autoload.as_ref(),
    );
    let proxy_deferred =
        detect_proxy_deferred_specs(&class_map, &factories, composer_autoload.as_ref());

    let mut interception_di_config = full_di_config.clone();
    interception_di_config.plugins = merge_plugins_for_interception(
        &scanner_di_configs[..global_di_configs.len()],
        &scanner_di_configs[global_di_configs.len()..],
    );

    // Interceptors can target generated factory classes (e.g. plugins on
    // Magento\Setup\...\EntityGeneratorFactory). Magento sees those classes
    // after factory generation; mirror that by adding synthetic class metadata
    // before interceptor detection.
    let mut interception_class_map = class_map.clone();
    augment_with_composer_plugin_owner_classes(
        &mut interception_class_map,
        &interception_di_config,
        composer_autoload.as_ref(),
    );
    for spec in &factories {
        if interception_class_map.contains_key(&spec.factory_fqcn) {
            continue;
        }
        interception_class_map.insert(
            spec.factory_fqcn.clone(),
            synthetic_factory_class_info(&spec.factory_fqcn),
        );
    }
    for spec in &search_results {
        if interception_class_map.contains_key(&spec.result_fqcn) {
            continue;
        }
        interception_class_map.insert(
            spec.result_fqcn.clone(),
            synthetic_search_results_class_info(spec),
        );
    }
    for spec in &proxy_deferred {
        if interception_class_map.contains_key(&spec.proxy_fqcn) {
            continue;
        }
        interception_class_map.insert(
            spec.proxy_fqcn.clone(),
            synthetic_proxy_deferred_class_info(spec),
        );
    }

    let mut interceptors = detect_interceptors(&interception_class_map, &interception_di_config);
    // PHP registers setup/src as type SETUP (not MODULE). It does not generate
    // interceptors for Magento\Setup\* classes that only inherit plugins (Phase 2
    // inheritance from e.g. Symfony\Console\Command\Command). However, if a Setup
    // class has a *direct* plugin declared in di.xml, PHP does generate its interceptor
    // (e.g. Magento\Setup\Model\FixtureGenerator\EntityGeneratorFactory). Keep those.
    interceptors.retain(|spec| {
        let fqcn = spec.fqcn.trim_start_matches('\\');
        if !fqcn.starts_with("Magento\\Setup\\") {
            return true; // non-Setup class — always keep
        }
        !spec.plugins.is_empty() // Setup class with direct plugins — keep; inherited-only — drop
    });
    enrich_interceptor_specs_with_reflection(
        &mut interceptors,
        &interception_class_map,
        &interception_di_config,
        &magento_root,
        &args.fallback_php,
    );

    let extension_interfaces_to_generate = extension_specs
        .iter()
        .filter(|spec| !class_map.contains_key(&spec.extension_interface_fqcn))
        .count();
    let extension_classes_to_generate = extension_specs
        .iter()
        .filter(|spec| !class_map.contains_key(&spec.extension_class_fqcn))
        .count();

    log::info!(
        "Detected: {} interceptors, {} factories, {} proxies, {} searchResults, {} proxyDeferred, {} extension interfaces, {} extension classes",
        interceptors.len(),
        factories.len(),
        proxies.len(),
        search_results.len(),
        proxy_deferred.len(),
        extension_interfaces_to_generate,
        extension_classes_to_generate,
    );
    log_phase_elapsed("Phase 4", phase_4_started);

    // -----------------------------------------------------------------------
    // Phase 5: Resolve arguments (global config for per-area override later)
    // -----------------------------------------------------------------------
    let phase_5_started = Instant::now();

    // Build PHP class-constant resolution map.
    // di.xml xsi:type="init_parameter" values contain PHP constant expressions like
    // `Magento\Framework\App\State::PARAM_MODE`. We resolve these by reading the
    // class source file and extracting the actual string constant value.
    //
    // TKT-054: Bootstrap with PHP extension constants first (e.g. MCRYPT_BLOWFISH,
    // MCRYPT_MODE_ECB) that are never defined in any Magento PHP source file.
    // Source-scan constants added after will override these builtins on collision.
    let const_map: FxHashMap<String, String> = {
        // Collect unique ClassName::CONST_NAME expressions from all di_config arguments
        let mut init_exprs: FxHashSet<String> = FxHashSet::default();
        let mut collect_from_arg = |arg: &Argument| {
            if let Argument::Init { value, .. } = arg {
                let normalized = value.trim().trim_start_matches('\\');
                if !normalized.is_empty() {
                    init_exprs.insert(normalized.to_string());
                }
            }
        };
        for tc in di_config.type_configs.values() {
            for arg in &tc.arguments {
                collect_from_arg(arg);
            }
        }

        // Bootstrap PHP extension/built-in constants as baseline.
        let mut map = bootstrap_php_constants(&args.fallback_php, &magento_root);

        // Source-scan: resolve ClassName::CONST_NAME expressions from PHP source files.
        // These override any bootstrap builtins with the same key.
        for expr in &init_exprs {
            let Some((class_name, const_name)) = expr.split_once("::") else {
                continue;
            };
            let Some(info) = class_map.get(class_name) else {
                continue;
            };
            let constants = extract_string_constants(&info.path);
            if let Some(value) = constants.get(const_name) {
                map.insert(expr.clone(), value.clone());
            }
        }
        map
    };
    log::info!(
        "Resolved {} PHP constant expressions for init_parameter",
        const_map.len()
    );

    let args_map = resolve_all_arguments(&class_map, &di_config, &const_map); // global only; per-area overrides applied later
    log::info!("Resolved arguments for {} classes", args_map.len());

    // Build all_fqcns map for interception.php (all FQCNs → bool intercepted)
    let intercepted_set: FxHashSet<&str> =
        interceptors.iter().map(|s| s.fqcn.as_str()).collect();
    let all_fqcns_phase5: FxHashMap<String, bool> = interception_class_map
        .keys()
        .map(|fqcn| {
            let intercepted = intercepted_set.contains(fqcn.as_str());
            (fqcn.clone(), intercepted)
        })
        .collect();
    log_phase_elapsed("Phase 5", phase_5_started);

    if args.dry_run {
        log::info!("Dry run — skipping file writes");
        print_summary(
            &interceptors,
            &factories,
            &proxies,
            &args_map,
            &all_fqcns_phase5,
        );
        log_phase_elapsed("Total", total_started);
        return;
    }

    // -----------------------------------------------------------------------
    // Phase 6: Generate PHP code files (parallel)
    // -----------------------------------------------------------------------
    let phase_6_started = Instant::now();
    let pb = progress_bar(
        (interceptors.len()
            + factories.len()
            + proxies.len()
            + search_results.len()
            + proxy_deferred.len()
            + extension_interfaces_to_generate
            + extension_classes_to_generate) as u64,
        "Generating code",
    );
    let written = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reflected_ctor_params: FxHashMap<String, Vec<ConstructorParam>> = interceptors
        .par_iter()
        .filter_map(|spec| {
            let info = interceptor_target_info_with_inherited_constructor(
                &spec.fqcn,
                &interception_class_map,
            )?;
            let needs_reflection = match info.constructor.as_ref() {
                Some(constructor) => constructor_params_need_reflection(&constructor.params),
                None => true,
            };
            if !needs_reflection {
                return None;
            }
            let params =
                reflect_constructor_params(&spec.fqcn, &args.magento_root, &args.fallback_php)?;
            Some((spec.fqcn.clone(), params))
        })
        .collect();
    let reflected_ctor_params = Arc::new(reflected_ctor_params);

    // Interceptors
    interceptors.par_iter().for_each(|spec| {
        let mut effective_info =
            interceptor_target_info_with_inherited_constructor(&spec.fqcn, &interception_class_map);
        if let (Some(info), Some(params)) = (
            effective_info.as_mut(),
            reflected_ctor_params.get(&spec.fqcn),
        ) {
            info.constructor = Some(Constructor {
                params: params.clone(),
            });
        }
        let content = generate_interceptor(spec, effective_info.as_ref());
        let rel = interceptor_path(&spec.fqcn);
        let out_path = code_root.join(&rel);
        if let Ok(changed) = write_if_changed(&out_path, &content) {
            if changed {
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });

    // Factories
    factories.par_iter().for_each(|spec| {
        let content = generate_factory(spec);
        let rel = factory_path(&spec.factory_fqcn);
        let out_path = code_root.join(&rel);
        if let Ok(changed) = write_if_changed(&out_path, &content) {
            if changed {
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });

    let unique_proxy_targets: Vec<String> = {
        let mut seen = FxHashSet::default();
        proxies
            .iter()
            .filter_map(|spec| {
                if seen.insert(spec.target_fqcn.clone()) {
                    Some(spec.target_fqcn.clone())
                } else {
                    None
                }
            })
            .collect()
    };
    let reflected_proxy_kinds: FxHashMap<String, ClassKind> = unique_proxy_targets
        .par_iter()
        .filter_map(|target_fqcn| {
            if interception_class_map.contains_key(target_fqcn) {
                return None;
            }
            let kind = reflect_class_kind(target_fqcn, &args.magento_root, &args.fallback_php)?;
            Some((target_fqcn.clone(), kind))
        })
        .collect();
    let reflected_proxy_methods: FxHashMap<String, Vec<MethodSignature>> = unique_proxy_targets
        .par_iter()
        .filter_map(|target_fqcn| {
            let mut reflected_methods =
                reflect_interceptable_methods(target_fqcn, &args.magento_root, &args.fallback_php)?;
            for method in reflected_methods.iter_mut() {
                normalize_reflected_method_signature_for_proxy(
                    method,
                    target_fqcn,
                    &interception_class_map,
                );
            }
            Some((target_fqcn.clone(), reflected_methods))
        })
        .collect();
    log::info!(
        "Proxy reflection precompute: {} reflected targets, {} reflected kinds ({} unique targets)",
        reflected_proxy_methods.len(),
        reflected_proxy_kinds.len(),
        unique_proxy_targets.len()
    );
    let reflected_proxy_methods = Arc::new(reflected_proxy_methods);
    let reflected_proxy_kinds = Arc::new(reflected_proxy_kinds);

    // Proxies
    proxies.par_iter().for_each(|spec| {
        let mut target_info =
            target_info_with_inherited_public_methods(&spec.target_fqcn, &interception_class_map);
        if target_info.is_none() {
            if let Some(kind) = reflected_proxy_kinds.get(&spec.target_fqcn) {
                target_info = Some(synthetic_proxy_target_info(&spec.target_fqcn, kind.clone()));
            }
        }
        if let Some(info) = target_info.as_mut() {
            if let Some(reflected_methods) = reflected_proxy_methods.get(&spec.target_fqcn) {
                // Reflection order/surface is Magento's source of truth for
                // proxy methods (declaration order including inherited publics).
                let mut methods = reflected_methods.clone();
                if let Some(pos) = info
                    .public_methods
                    .iter()
                    .position(|m| m.name == "_resetState")
                {
                    if !methods.iter().any(|m| m.name == "_resetState") {
                        methods.insert(
                            pos.min(methods.len()),
                            MethodSignature {
                                name: "_resetState".to_string(),
                                params: vec![],
                                return_type: Some("void".to_string()),
                                is_static: false,
                                returns_reference: false,
                            },
                        );
                    }
                }
                info.public_methods = methods;
            }
        }
        let content = generate_proxy(spec, target_info.as_ref());
        let rel = proxy_path(&spec.proxy_fqcn);
        let out_path = code_root.join(&rel);
        if let Ok(changed) = write_if_changed(&out_path, &content) {
            if changed {
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });

    // Search results
    search_results.par_iter().for_each(|spec| {
        let content = generate_search_results(&spec.result_fqcn, &spec.source_fqcn);
        let rel = search_results_path(&spec.result_fqcn);
        let out_path = code_root.join(&rel);
        if let Ok(changed) = write_if_changed(&out_path, &content) {
            if changed {
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });

    // Proxy deferred
    proxy_deferred.par_iter().for_each(|spec| {
        let target_info =
            target_info_with_inherited_public_methods(&spec.target_fqcn, &interception_class_map);
        let content =
            generate_proxy_deferred(&spec.proxy_fqcn, &spec.target_fqcn, target_info.as_ref());
        let rel = proxy_deferred_path(&spec.proxy_fqcn);
        let out_path = code_root.join(&rel);
        if let Ok(changed) = write_if_changed(&out_path, &content) {
            if changed {
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });

    // Extension interfaces
    extension_specs.par_iter().for_each(|spec| {
        if class_map.contains_key(&spec.extension_interface_fqcn) {
            return;
        }
        let content = generate_extension_interface(spec);
        let rel = extension_path(&spec.extension_interface_fqcn);
        let out_path = code_root.join(&rel);
        if let Ok(changed) = write_if_changed(&out_path, &content) {
            if changed {
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });

    // Extension classes
    extension_specs.par_iter().for_each(|spec| {
        if class_map.contains_key(&spec.extension_class_fqcn) {
            return;
        }
        let content = generate_extension(spec);
        let rel = extension_path(&spec.extension_class_fqcn);
        let out_path = code_root.join(&rel);
        if let Ok(changed) = write_if_changed(&out_path, &content) {
            if changed {
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });
    pb.finish_with_message("done");
    log_phase_elapsed("Phase 6", phase_6_started);

    // -----------------------------------------------------------------------
    // Phase 7: Generate metadata files
    // -----------------------------------------------------------------------
    let phase_7_started = Instant::now();
    log::info!("Generating metadata files");

    // Metadata type universe: include real classes, generated classes, and
    // DI-declared logical types (virtual types/preferences/plugins/type configs).
    let resolved_const_values =
        resolve_php_constants_in_config(&full_di_config, &args.magento_root, &args.fallback_php);
    let mut metadata_base_di_config = di_config.clone();
    // setup:di:compile mutates ObjectManager config at runtime before metadata
    // generation. Mirror the relevant overrides so Setup/compiler classes and
    // scanner exclusions match PHP metadata output.
    apply_setup_di_compile_runtime_overrides(
        &mut metadata_base_di_config,
        &args.magento_root,
        &module_paths,
        &args.fallback_php,
    );
    apply_resolved_constants_to_di_config(&mut metadata_base_di_config, &resolved_const_values);
    // Freeze into Arc so the area par_iter loop can share it without cloning.
    let metadata_base_di_config = std::sync::Arc::new(metadata_base_di_config);

    let generated_class_map = extract_generated_class_map(&code_root);
    let mut metadata_class_map = merged_class_map(&interception_class_map, &generated_class_map);
    // Source-only FQCNs for null-emission filtering: PHP emits NULL only for scanned
    // source concrete classes, not for generated artifacts (interceptors, factories, proxies).
    let base_class_fqcns: FxHashSet<String> = interception_class_map.keys().cloned().collect();
    let argument_type_names = build_argument_type_names(
        &interception_class_map,
        &generated_class_map,
        &metadata_base_di_config,
        &interceptors,
        &factories,
        &proxies,
        &search_results,
        &proxy_deferred,
        &extension_specs,
    );
    let interception_type_names = build_interception_type_names(
        &interception_class_map,
        &generated_class_map,
        &metadata_base_di_config,
        &interceptors,
        &factories,
        &proxies,
        &search_results,
        &proxy_deferred,
        &extension_specs,
    );
    // All three reflection passes merged into one par_iter — eliminates two
    // Rayon barriers. Candidates are disjoint: pass 1 has constructors with
    // constant defaults, pass 2 has no constructor + extends, pass 3 adds
    // VT targets absent from class_map.
    let (reflected_metadata_ctors, reflected_inherited_ctors, reflected_virtual_target_ctors) =
        enrich_all_constructors_with_reflection(
            &mut metadata_class_map,
            &argument_type_names,
            &metadata_base_di_config,
            &args.magento_root,
            &args.fallback_php,
        );
    log::info!(
        "Metadata universe: args {} / interception {} type names (base {}, generated {}, ctor reflections {}, inherited reflections {}, virtual-target reflections {})",
        argument_type_names.len(),
        interception_type_names.len(),
        interception_class_map.len(),
        generated_class_map.len(),
        reflected_metadata_ctors,
        reflected_inherited_ctors,
        reflected_virtual_target_ctors
    );
    // Build direct interception map once; area-specific preference/instanceTypes
    // overrides are derived from this and each area's merged DI config.
    let global_interceptor_map = build_direct_interception_map(&interceptors);
    let all_fqcns = build_interception_registry(
        &interception_type_names,
        &interceptors,
        &proxies,
        &proxy_deferred,
        &interception_di_config,
        &metadata_class_map,
    );

    // interception.php
    let interception_content = serialize_interception_php(&all_fqcns);
    let interception_path = metadata_root.join("interception.php");
    let _ = write_if_changed(&interception_path, &interception_content);

    // Build the case-normalisation index once from the interception class map.
    // All 7 area iterations reuse the same index; previously it was rebuilt
    // from scratch inside canonicalize_instance_reference_case on every call.
    let case_index = build_case_index(&interception_class_map);

    // Per-area config files — each area merges global + area-specific di.xml overlays.
    // Run in parallel: each area is independent (different files, different output path).
    let t_area_config = Instant::now();
    let pb_area = progress_bar(AREAS.len() as u64, "Generating area configs");
    let area_di_configs: FxHashMap<String, std::sync::Arc<DiConfig>> = AREAS
        .par_iter()
        .map(|&area| {
            let area_di_files = filter_enabled_di_xml(
                find_di_xml_files_for_area(&magento_root, area, &enabled_modules),
                &enabled_modules,
                &magento_root,
            );

            // Only re-merge if there are area-specific files beyond the global set.
            // Use Arc::clone for the no-overrides path to avoid cloning the large DiConfig.
            let area_di_config: std::sync::Arc<DiConfig> =
                if area_di_files.len() > di_xml_files.len() {
                    let area_only: Vec<_> = area_di_files
                        .iter()
                        .filter(|p| !di_xml_files_set.contains(p))
                        .collect();
                    if area_only.is_empty() {
                        std::sync::Arc::clone(&metadata_base_di_config)
                    } else {
                        // Look up from Phase 3b cache; fall back to re-parsing only for
                        // files not in the cache (should not happen in practice).
                        let extra_configs: Vec<DiConfig> = area_only
                            .iter()
                            .filter_map(|p| {
                                area_di_xml_cache
                                    .get(*p)
                                    .cloned()
                                    .or_else(|| parse_di_xml(p).ok())
                            })
                            .collect();
                        // Area-specific module configs applied via the same two-phase merge as
                        // global: deep-merge area module configs together, then apply on the
                        // global base using shallow (array_replace) semantics per PHP behaviour.
                        let mut merged_area = apply_module_config_on_primary(
                            (*metadata_base_di_config).clone(),
                            merge_configs(extra_configs),
                        );
                        apply_resolved_constants_to_di_config(
                            &mut merged_area,
                            &resolved_const_values,
                        );
                        std::sync::Arc::new(merged_area)
                    }
                } else {
                    std::sync::Arc::clone(&metadata_base_di_config)
                };

            // Build area-specific preference overrides.
            // global_interceptor_map is shared read-only; no per-area clone needed.
            let area_preference_overrides =
                build_interception_preference_overrides(&area_di_config, &global_interceptor_map);

            let mut area_di_config_for_args = (*area_di_config).clone();
            for (from, to) in &area_preference_overrides {
                // Keep VT argument values stable: synthetic interception overrides for
                // virtual-type names should not participate in argument object resolution.
                // Explicit DI preferences for VT aliases are already present in
                // area_di_config.preferences and remain intact.
                if area_di_config_for_args
                    .virtual_types
                    .contains_key(from.as_str())
                {
                    continue;
                }
                area_di_config_for_args
                    .preferences
                    .insert(from.clone(), to.clone());
            }
            area_di_config_for_args.refresh_lookup_indexes();

            // Area-specific di.xml files may define additional virtual types not present
            // in the global di config. Extend the type universe with those for this area.
            let area_type_names: Vec<String> = {
                let base_set: FxHashSet<&str> =
                    argument_type_names.iter().map(|s| s.as_str()).collect();
                let extra: Vec<String> = area_di_config_for_args
                    .virtual_types
                    .keys()
                    .filter(|k| !base_set.contains(k.as_str()))
                    .cloned()
                    .collect();
                if extra.is_empty() {
                    argument_type_names.clone()
                } else {
                    let mut v = argument_type_names.clone();
                    v.extend(extra);
                    v
                }
            };
            let mut area_args = resolve_all_arguments_for_named_types(
                &area_type_names,
                &metadata_class_map,
                &base_class_fqcns,
                &area_di_config_for_args,
                &const_map,
            );
            // Reuse pre-built case index: avoids rebuilding from class_map on every area.
            apply_case_index(&mut area_args, &case_index);
            let area_args: FxHashMap<String, Vec<di_resolver::ResolvedArg>> = area_args
                .into_iter()
                .filter(|(fqcn, args)| {
                    // Intercepted concrete classes appear under ClassName\Interceptor, not
                    // their original name.  However virtual types must still appear even when
                    // their direct concrete type is intercepted — PHP generates arguments for
                    // both the VT name and the Interceptor. Exception: when intercepted
                    // source classes resolve to no constructor args, PHP keeps a class-level
                    // NULL entry for the original class name.
                    if !global_interceptor_map.contains_key(fqcn) {
                        return true;
                    }
                    if metadata_base_di_config
                        .virtual_types
                        .contains_key(fqcn.as_str())
                    {
                        return true;
                    }
                    if !args.is_empty() {
                        return false;
                    }
                    let normalized = fqcn.trim_start_matches('\\');
                    if !base_class_fqcns.contains(normalized) {
                        return false;
                    }
                    matches!(
                        metadata_class_map.get(normalized).map(|info| &info.kind),
                        Some(ClassKind::Class) | Some(ClassKind::Trait)
                    )
                })
                .collect();
            let area_content = generate_area_config_with_overrides(
                &area_args,
                &area_di_config,
                &area_preference_overrides,
                &global_interceptor_map,
            );
            let area_path = metadata_root.join(format!("{}.php", area));
            let _ = write_if_changed(&area_path, &area_content);
            pb_area.inc(1);
            (area.to_string(), area_di_config)
        })
        .collect();
    pb_area.finish_with_message("done");
    log::debug!("Phase 7 area-config loop: {:?}", t_area_config.elapsed());

    // Scope-specific plugin-list metadata files.
    let plugin_list_class_definitions: Vec<String> = Vec::new();
    let plugin_scopes = [
        "global",
        "adminhtml",
        "crontab",
        "frontend",
        "graphql",
        "webapi_rest",
        "webapi_soap",
    ];
    let t_plugin_list = Instant::now();
    let pb_plugins = progress_bar(
        plugin_scopes.len() as u64,
        "Generating plugin-list metadata",
    );
    for scope in plugin_scopes {
        if let Some(scope_di_config) = area_di_configs.get(scope) {
            // For non-global scopes virtual types are not emitted; pass the flag
            // directly instead of cloning + clearing the full DiConfig.
            let include_vt = scope == "global";
            let metadata = compile_plugin_list(
                &**scope_di_config,
                &class_map,
                &plugin_list_class_definitions,
                include_vt,
            );
            let content = serialize_plugin_list_php(&metadata);
            let cache_id = plugin_list_cache_id(scope);
            let path = metadata_root.join(format!("{}.php", cache_id));
            let _ = write_if_changed(&path, &content);
        }
        pb_plugins.inc(1);
    }
    pb_plugins.finish_with_message("done");
    log::debug!("Phase 7 plugin-list loop: {:?}", t_plugin_list.elapsed());

    // App action list metadata.
    let app_action_list = generate_app_action_list_php(&class_map);
    let app_action_list_path = metadata_root.join("app_action_list.php");
    let _ = write_if_changed(&app_action_list_path, &app_action_list);
    log_phase_elapsed("Phase 7", phase_7_started);

    let total_written = written.load(std::sync::atomic::Ordering::Relaxed);
    log::info!(
        "Code generation complete: {} files written, {} unchanged",
        total_written,
        interceptors.len()
            + factories.len()
            + proxies.len()
            + search_results.len()
            + proxy_deferred.len()
            + extension_interfaces_to_generate
            + extension_classes_to_generate
            - total_written
    );

    // Update incremental cache
    if args.incremental {
        for path in &all_di_xml_files {
            cache.record(path);
        }
        cache.save(&cache_path);
        log::debug!("Incremental cache saved to {}", cache_path.display());
    }

    if args.compare_archive {
        let compare_archive_started = Instant::now();
        let archive_root = args
            .archive_root
            .clone()
            .unwrap_or_else(|| magento_root.join("generated"));
        let report_dir = args
            .compare_report_dir
            .clone()
            .unwrap_or_else(|| generated_root.join("diff"));
        match compare_against_archive(
            &generated_root,
            &archive_root,
            &report_dir,
            &args.fallback_php,
        ) {
            Ok(summary) => {
                log::info!(
                    "Archive diff: code missing {}, code extra {}, code changed {}, metadata missing {}, metadata extra {}, metadata changed {} (reports: {})",
                    summary.code_missing,
                    summary.code_extra,
                    summary.code_changed,
                    summary.metadata_missing,
                    summary.metadata_extra,
                    summary.metadata_changed,
                    report_dir.display()
                );
                if args.compare_fail_on_diff && !summary.is_clean() {
                    log_phase_elapsed("Total", total_started);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("error: archive compare failed: {e}");
                log_phase_elapsed("Total", total_started);
                std::process::exit(2);
            }
        }
        log_phase_elapsed("Archive compare", compare_archive_started);
    }

    // -----------------------------------------------------------------------
    // Phase 8: Validation (optional)
    // -----------------------------------------------------------------------
    if args.validate {
        let phase_8_started = Instant::now();
        // --php-generated is required (enforced earlier in main)
        let php_gen = args.php_generated.as_deref().unwrap();
        log::info!("Validating against {}", php_gen.display());
        let result = validator::validate(php_gen, &generated_root);
        println!("{}", result.summary());
        log_phase_elapsed("Phase 8", phase_8_started);
        if !result.is_clean() {
            log_phase_elapsed("Total", total_started);
            std::process::exit(1);
        }
    }

    log_phase_elapsed("Total", total_started);
    log::info!("fast-di-compile finished successfully");

    // Clean up the temp PHP worker script.
    let _ = std::fs::remove_file(&worker_script_path);
}

fn progress_bar(len: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg} [{bar:40}] {pos}/{len} {elapsed}")
            .unwrap()
            .progress_chars("=> "),
    );
    pb.set_message(msg.to_string());
    pb
}

fn log_phase_elapsed(phase: &str, started: Instant) {
    let elapsed = started.elapsed();
    log::info!(
        "{} elapsed: {:.3}s ({} ms)",
        phase,
        elapsed.as_secs_f64(),
        elapsed.as_millis()
    );
}

#[derive(Clone, Debug)]
enum ResolvedConstValue {
    String(String),
    Number(String),
    Bool(bool),
    Null,
}

fn resolve_php_constants_in_config(
    di_config: &DiConfig,
    magento_root: &Path,
    php_bin: &str,
) -> FxHashMap<String, ResolvedConstValue> {
    let mut const_exprs = FxHashSet::default();
    for type_config in di_config.type_configs.values() {
        collect_const_expressions(&type_config.arguments, &mut const_exprs);
    }
    if const_exprs.is_empty() {
        return FxHashMap::default();
    }

    let const_vec: Vec<String> = const_exprs.into_iter().collect();
    const_vec
        .par_iter()
        .filter_map(|expr| {
            let value = reflect_constant_value(expr, magento_root, php_bin)?;
            Some((expr.clone(), value))
        })
        .collect()
}

fn collect_const_expressions(arguments: &[Argument], out: &mut FxHashSet<String>) {
    for arg in arguments {
        match arg {
            Argument::Const { value, .. } => {
                let normalized = value.trim().trim_start_matches('\\');
                if !normalized.is_empty() {
                    out.insert(normalized.to_string());
                }
            }
            Argument::Array { items, .. } => collect_const_expressions(items, out),
            _ => {}
        }
    }
}

fn reflect_constant_value(
    expr: &str,
    _magento_root: &Path,
    _php_bin: &str,
) -> Option<ResolvedConstValue> {
    let pool = PHP_WORKER_POOL.get()?;
    let request = format!("const:{}", expr.trim().trim_start_matches('\\'));
    let json_raw = pool.request(&request)?;
    let json_trimmed = json_raw.trim();
    if json_trimmed == "null" || json_trimmed.is_empty() {
        return Some(ResolvedConstValue::Null);
    }
    let value: serde_json::Value = serde_json::from_str(json_trimmed).ok()?;
    match value {
        serde_json::Value::String(s) => Some(ResolvedConstValue::String(s)),
        serde_json::Value::Number(n) => Some(ResolvedConstValue::Number(n.to_string())),
        serde_json::Value::Bool(b) => Some(ResolvedConstValue::Bool(b)),
        serde_json::Value::Null => Some(ResolvedConstValue::Null),
        _ => None,
    }
}

fn apply_resolved_constants_to_di_config(
    di_config: &mut DiConfig,
    values: &FxHashMap<String, ResolvedConstValue>,
) {
    if values.is_empty() {
        return;
    }
    for type_config in di_config.type_configs.values_mut() {
        apply_resolved_constants_to_arguments(&mut type_config.arguments, values);
    }
}

fn apply_resolved_constants_to_arguments(
    arguments: &mut [Argument],
    values: &FxHashMap<String, ResolvedConstValue>,
) {
    for arg in arguments.iter_mut() {
        match arg {
            Argument::Const {
                name,
                value,
                sort_order,
            } => {
                let key = value.trim().trim_start_matches('\\').to_string();
                let so = *sort_order;
                if let Some(resolved) = values.get(&key) {
                    *arg = match resolved {
                        ResolvedConstValue::String(v) => Argument::String {
                            name: name.clone(),
                            value: v.clone(),
                            sort_order: so,
                        },
                        ResolvedConstValue::Number(v) => Argument::Number {
                            name: name.clone(),
                            value: v.clone(),
                            sort_order: so,
                        },
                        ResolvedConstValue::Bool(v) => Argument::Boolean {
                            name: name.clone(),
                            value: *v,
                            sort_order: so,
                        },
                        ResolvedConstValue::Null => Argument::Null {
                            name: name.clone(),
                            sort_order: so,
                        },
                    };
                }
            }
            Argument::Array { items, .. } => apply_resolved_constants_to_arguments(items, values),
            _ => {}
        }
    }
}

fn apply_setup_di_compile_runtime_overrides(
    di_config: &mut DiConfig,
    magento_root: &Path,
    module_paths: &[PathBuf],
    php_bin: &str,
) {
    // configureObjectManager(): ClassesScanner.excludePatterns
    apply_setup_classes_scanner_exclude_patterns(di_config, magento_root, module_paths, php_bin);
    // configureObjectManager(): ModificationChain.modificationsList
    apply_setup_modification_chain_override(di_config);
    // configureObjectManager(): PluginList.cache -> CompiledConfig (no Proxy)
    apply_setup_plugin_list_cache_override(di_config);
    di_config.refresh_lookup_indexes();
}

fn apply_setup_classes_scanner_exclude_patterns(
    di_config: &mut DiConfig,
    magento_root: &Path,
    module_paths: &[PathBuf],
    php_bin: &str,
) {
    // Prefer PHP runtime registrar paths to mirror setup:di:compile exactly.
    // Fallback to local discovery if bootstrap lookup fails.
    let (module_roots, library_paths, setup_root) =
        if let Some(paths) = load_setup_component_paths_from_php(magento_root, php_bin) {
            (
                paths.module_paths,
                paths.library_paths,
                PathBuf::from(paths.setup_path),
            )
        } else {
            let setup_root = magento_root.join("setup");
            let library_paths = discover_framework_library_paths(magento_root);
            let module_roots: Vec<PathBuf> = module_paths
                .iter()
                .filter(|p| !p.starts_with(&setup_root))
                .filter(|p| !library_paths.iter().any(|lib| *p == lib))
                .cloned()
                .collect();
            (module_roots, library_paths, setup_root)
        };

    let application_patterns = build_setup_excluded_module_patterns(&module_roots);
    let framework_patterns = build_setup_excluded_library_patterns(&library_paths);
    let setup_patterns = vec![format!(
        "#^(?:{})(/[\\w]+)*/Test#",
        regex_quote_for_hash(&setup_root.to_string_lossy())
    )];

    let exclude_patterns = Argument::Array {
        name: "excludePatterns".to_string(),
        items: vec![
            string_list_array_arg("application", &application_patterns),
            string_list_array_arg("framework", &framework_patterns),
            string_list_array_arg("setup", &setup_patterns),
        ],
        sort_order: 0,
    };

    upsert_type_argument(
        di_config,
        "Magento\\Setup\\Module\\Di\\Code\\Reader\\ClassesScanner",
        exclude_patterns,
    );
}

fn apply_setup_modification_chain_override(di_config: &mut DiConfig) {
    let modifications_list = Argument::Array {
        name: "modificationsList".to_string(),
        items: vec![
            object_item_arg(
                "BackslashTrim",
                "Magento\\Setup\\Module\\Di\\Compiler\\Config\\Chain\\BackslashTrim",
            ),
            object_item_arg(
                "PreferencesResolving",
                "Magento\\Setup\\Module\\Di\\Compiler\\Config\\Chain\\PreferencesResolving",
            ),
            object_item_arg(
                "InterceptorSubstitution",
                "Magento\\Setup\\Module\\Di\\Compiler\\Config\\Chain\\InterceptorSubstitution",
            ),
            object_item_arg(
                "InterceptionPreferencesResolving",
                "Magento\\Setup\\Module\\Di\\Compiler\\Config\\Chain\\PreferencesResolving",
            ),
        ],
        sort_order: 0,
    };

    upsert_type_argument(
        di_config,
        "Magento\\Setup\\Module\\Di\\Compiler\\Config\\ModificationChain",
        modifications_list,
    );
}

fn apply_setup_plugin_list_cache_override(di_config: &mut DiConfig) {
    upsert_type_argument(
        di_config,
        "Magento\\Setup\\Module\\Di\\Code\\Generator\\PluginList",
        Argument::Object {
            name: "cache".to_string(),
            value: "Magento\\Framework\\App\\Interception\\Cache\\CompiledConfig".to_string(),
            shared: None,
            sort_order: 0,
        },
    );
}

fn upsert_type_argument(di_config: &mut DiConfig, type_name: &str, argument: Argument) {
    let tc = di_config
        .type_configs
        .entry(type_name.to_string())
        .or_default();
    let arg_name = match &argument {
        Argument::Object { name, .. }
        | Argument::String { name, .. }
        | Argument::Boolean { name, .. }
        | Argument::Number { name, .. }
        | Argument::Null { name, .. }
        | Argument::Array { name, .. }
        | Argument::Init { name, .. }
        | Argument::Const { name, .. } => name,
    };

    if let Some(existing) = tc.arguments.iter_mut().find(|a| match a {
        Argument::Object { name, .. }
        | Argument::String { name, .. }
        | Argument::Boolean { name, .. }
        | Argument::Number { name, .. }
        | Argument::Null { name, .. }
        | Argument::Array { name, .. }
        | Argument::Init { name, .. }
        | Argument::Const { name, .. } => name == arg_name,
    }) {
        *existing = argument;
    } else {
        tc.arguments.push(argument);
    }
}

fn object_item_arg(name: &str, fqcn: &str) -> Argument {
    Argument::Object {
        name: name.to_string(),
        value: fqcn.to_string(),
        shared: None,
        sort_order: 0,
    }
}

fn string_list_array_arg(name: &str, values: &[String]) -> Argument {
    Argument::Array {
        name: name.to_string(),
        items: values
            .iter()
            .enumerate()
            .map(|(idx, value)| Argument::String {
                name: idx.to_string(),
                value: value.clone(),
                sort_order: 0,
            })
            .collect(),
        sort_order: 0,
    }
}

fn build_setup_excluded_module_patterns(module_roots: &[PathBuf]) -> Vec<String> {
    let mut modules_by_base: Vec<(String, Vec<(String, Vec<String>)>)> = Vec::new();

    for module_root in module_roots {
        let Some(module_dir) = module_root.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(vendor_path) = module_root.parent() else {
            continue;
        };
        let Some(vendor_dir) = vendor_path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(base_path) = vendor_path.parent() else {
            continue;
        };

        let base_key = base_path.to_string_lossy().to_string();
        let base_idx = modules_by_base
            .iter()
            .position(|(base, _)| base == &base_key)
            .unwrap_or_else(|| {
                modules_by_base.push((base_key.clone(), Vec::new()));
                modules_by_base.len() - 1
            });

        let vendors = &mut modules_by_base[base_idx].1;
        let vendor_key = vendor_dir.to_string();
        let vendor_idx = vendors
            .iter()
            .position(|(vendor, _)| vendor == &vendor_key)
            .unwrap_or_else(|| {
                vendors.push((vendor_key.clone(), Vec::new()));
                vendors.len() - 1
            });

        let modules = &mut vendors[vendor_idx].1;
        let module_name = module_dir.to_string();
        if !modules.iter().any(|m| m == &module_name) {
            modules.push(module_name);
        }
    }

    if modules_by_base.is_empty() {
        return Vec::new();
    }

    let mut base_paths_regexps: Vec<String> = Vec::new();
    for (base_path, vendor_paths) in modules_by_base {
        let vendor_paths_regexps: Vec<String> = vendor_paths
            .into_iter()
            .filter_map(|(vendor_dir, vendor_modules)| {
                if vendor_modules.is_empty() {
                    return None;
                }
                Some(format!("{}/(?:{})", vendor_dir, vendor_modules.join("|")))
            })
            .collect();
        if vendor_paths_regexps.is_empty() {
            continue;
        }

        base_paths_regexps.push(format!(
            "{}/(?:{})",
            regex_quote_for_hash(&base_path),
            vendor_paths_regexps.join("|")
        ));
    }
    if base_paths_regexps.is_empty() {
        return Vec::new();
    }

    vec![
        format!("#^(?:{})/Test#", base_paths_regexps.join("|")),
        format!("#^(?:{})/tests#", base_paths_regexps.join("|")),
    ]
}

fn build_setup_excluded_library_patterns(library_paths: &[PathBuf]) -> Vec<String> {
    if library_paths.is_empty() {
        return Vec::new();
    }
    let library_paths = library_paths
        .iter()
        .map(|p| regex_quote_for_hash(&p.to_string_lossy()))
        .collect::<Vec<_>>()
        .join("|");
    vec![
        format!("#^(?:{})/([\\w]+/)?Test#", library_paths),
        format!("#^(?:{})/([\\w]+/)?tests#", library_paths),
    ]
}

fn discover_framework_library_paths(magento_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let vendor_magento = magento_root.join("vendor/magento");
    if let Ok(entries) = std::fs::read_dir(vendor_magento) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            if name == "framework" || name.starts_with("framework-") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[derive(Debug, Deserialize)]
struct SetupComponentPaths {
    modules: Vec<String>,
    libraries: Vec<String>,
    setup: String,
}

struct SetupComponentPathsResolved {
    module_paths: Vec<PathBuf>,
    library_paths: Vec<PathBuf>,
    setup_path: String,
}

fn load_setup_component_paths_from_php(
    magento_root: &Path,
    php_bin: &str,
) -> Option<SetupComponentPathsResolved> {
    let script = r#"
$root = rtrim((string)($argv[1] ?? ''), '/');
if ($root === '' || !is_file($root . '/app/bootstrap.php')) {
    echo 'null';
    return;
}
require $root . '/app/bootstrap.php';
$registrar = new \Magento\Framework\Component\ComponentRegistrar();
$modules = array_values($registrar->getPaths(\Magento\Framework\Component\ComponentRegistrar::MODULE));
$libraries = array_values($registrar->getPaths(\Magento\Framework\Component\ComponentRegistrar::LIBRARY));
$setupPath = (new \Magento\Framework\App\Filesystem\DirectoryList($root))
    ->getPath(\Magento\Framework\App\Filesystem\DirectoryList::SETUP);
echo json_encode(
    ['modules' => $modules, 'libraries' => $libraries, 'setup' => $setupPath],
    JSON_UNESCAPED_SLASHES
);
"#;

    let output = Command::new(php_bin)
        .arg("-r")
        .arg(script)
        .arg(magento_root.as_os_str())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    let json = if trimmed.starts_with('{') {
        trimmed
    } else {
        let start = trimmed.find('{')?;
        let end = trimmed.rfind('}')?;
        if end < start {
            return None;
        }
        &trimmed[start..=end]
    };
    let parsed: SetupComponentPaths = serde_json::from_str(json).ok()?;
    Some(SetupComponentPathsResolved {
        module_paths: parsed.modules.into_iter().map(PathBuf::from).collect(),
        library_paths: parsed.libraries.into_iter().map(PathBuf::from).collect(),
        setup_path: parsed.setup,
    })
}

fn regex_quote_for_hash(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        match ch {
            '\\' | '.' | '+' | '*' | '?' | '[' | '^' | ']' | '$' | '(' | ')' | '{' | '}' | '='
            | '!' | '<' | '>' | '|' | ':' | '-' | '#' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Merged single-pass replacement for the three formerly sequential reflection functions.
///
/// Collects candidates from all three passes in one sequential scan, then
/// dispatches a single `par_iter()` call over the unified list — eliminating
/// two Rayon work-stealing barriers compared to the original three separate
/// `par_iter()` calls.
///
/// Returns `(ctor_defaults_reflected, inherited_reflected, vt_target_reflected)`.
fn enrich_all_constructors_with_reflection(
    class_map: &mut FxHashMap<String, ClassInfo>,
    argument_type_names: &[String],
    di_config: &DiConfig,
    magento_root: &Path,
    php_bin: &str,
) -> (usize, usize, usize) {
    // kind: 0 = update ctor (allow empty), 1 = update ctor (skip empty), 2 = insert if missing
    let mut candidates: Vec<(String, u8)> = Vec::new();

    // Pass 1: classes whose existing ctor has constant-reference defaults.
    for (fqcn, info) in class_map.iter() {
        let needs = info
            .constructor
            .as_ref()
            .map(|ctor| constructor_defaults_need_constant_reflection(&ctor.params))
            .unwrap_or(false);
        if needs {
            candidates.push((fqcn.clone(), 0));
        }
    }

    // Pass 2: argument-universe classes with no ctor that inherit from something
    // outside our scan scope.
    for fqcn in argument_type_names {
        if let Some(info) = class_map.get(fqcn) {
            if info.constructor.is_none() && !info.is_abstract && info.extends.is_some() {
                candidates.push((fqcn.clone(), 1));
            }
        }
    }

    // Pass 3: concrete targets of virtual types that are absent from class_map
    // (e.g. Monolog\Logger or other third-party classes).
    let vt_targets: FxHashSet<String> = argument_type_names
        .iter()
        .filter_map(|name| {
            let vt_name = name.trim_start_matches('\\');
            if !di_config.virtual_types.contains_key(vt_name) {
                return None;
            }
            let target = di_config.get_instance_type(vt_name);
            if target == vt_name || class_map.contains_key(&target) {
                return None;
            }
            Some(target)
        })
        .collect();
    for target in vt_targets {
        candidates.push((target, 2));
    }

    if candidates.is_empty() {
        return (0, 0, 0);
    }

    // Single par_iter over all candidates — one Rayon barrier instead of three.
    let reflected: Vec<(String, Vec<ConstructorParam>, u8)> = candidates
        .par_iter()
        .filter_map(|(fqcn, kind)| {
            let params = reflect_constructor_params(fqcn, magento_root, php_bin)?;
            if *kind == 1 && params.is_empty() {
                return None; // inherited no-arg ctor is not useful
            }
            Some((fqcn.clone(), params, *kind))
        })
        .collect();

    let (mut count0, mut count1, mut count2) = (0usize, 0usize, 0usize);
    for (fqcn, params, kind) in reflected {
        if kind == 2 {
            class_map
                .entry(fqcn.clone())
                .or_insert_with(|| synthetic_reflected_class_info(&fqcn, params));
            count2 += 1;
        } else if let Some(info) = class_map.get_mut(&fqcn) {
            info.constructor = Some(Constructor { params });
            if kind == 0 {
                count0 += 1;
            } else {
                count1 += 1;
            }
        }
    }

    (count0, count1, count2)
}

fn synthetic_reflected_class_info(fqcn: &str, params: Vec<ConstructorParam>) -> ClassInfo {
    let normalized = fqcn.trim_start_matches('\\');
    let (namespace, name) = if let Some((ns, class_name)) = normalized.rsplit_once('\\') {
        (ns.to_string(), class_name.to_string())
    } else {
        (String::new(), normalized.to_string())
    };

    ClassInfo {
        path: PathBuf::from("__reflected__/constructor.php"),
        namespace,
        name,
        fqcn: normalized.to_string(),
        kind: ClassKind::Class,
        extends: None,
        implements: vec![],
        constructor: Some(Constructor { params }),
        is_abstract: false,
        is_final: false,
        public_methods: vec![],
    }
}

fn constructor_defaults_need_constant_reflection(params: &[ConstructorParam]) -> bool {
    params.iter().any(|p| {
        p.default_value
            .as_deref()
            .map(|dv| dv.contains("::"))
            .unwrap_or(false)
    })
}

fn extract_generated_class_map(code_root: &Path) -> FxHashMap<String, ClassInfo> {
    if !code_root.is_dir() {
        return FxHashMap::default();
    }
    let files = walk_php_files(&[code_root.to_path_buf()]);
    let extracted: Vec<(String, ClassInfo)> = files
        .par_iter()
        .filter_map(|path| match extract_file(path) {
            ExtractResult::Ok(info) => Some((info.fqcn.clone(), info)),
            _ => None,
        })
        .collect();

    let mut map = FxHashMap::default();
    map.reserve(extracted.len());
    for (fqcn, info) in extracted {
        map.insert(fqcn, info);
    }
    map
}

fn merged_class_map(
    base: &FxHashMap<String, ClassInfo>,
    extra: &FxHashMap<String, ClassInfo>,
) -> FxHashMap<String, ClassInfo> {
    let mut out = base.clone();
    for (fqcn, info) in extra {
        out.insert(fqcn.clone(), info.clone());
    }
    out
}

fn build_argument_type_names(
    base_class_map: &FxHashMap<String, ClassInfo>,
    _generated_class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
    interceptors: &[di_resolver::InterceptorSpec],
    factories: &[di_resolver::FactorySpec],
    proxies: &[di_resolver::ProxySpec],
    search_results: &[SearchResultsSpec],
    proxy_deferred: &[ProxyDeferredSpec],
    extension_specs: &[ExtensionSpec],
) -> Vec<String> {
    let mut names: FxHashSet<String> = FxHashSet::default();

    // Include all scanned source classes (abstract and concrete). Intercepted classes
    // are included so class-level NULL owner entries can be emitted when appropriate.
    // PHP's DI compiler includes abstract classes in the arguments universe.
    // Pre-existing generated classes (interceptors, factories, proxies) are NOT
    // added here; only explicitly-detected generated classes (via spec lists below)
    // are included — adding all generated_class_map keys causes pre-existing
    // generated artifacts that PHP never re-processes to appear as extras.
    names.extend(base_class_map.keys().cloned());
    // Virtual types are part of the argument metadata universe regardless of whether
    // their direct target type is intercepted.
    names.extend(di_config.virtual_types.keys().cloned());
    // type_configs: include all configured names.
    names.extend(di_config.type_configs.keys().cloned());

    for spec in interceptors {
        let target = spec.fqcn.trim_start_matches('\\').to_string();
        // Only the Interceptor variant appears in arguments, not the concrete class itself.
        names.insert(format!("{target}\\Interceptor"));
    }

    for spec in factories {
        names.insert(spec.target_fqcn.clone());
        names.insert(spec.factory_fqcn.clone());
    }
    for spec in proxies {
        names.insert(spec.target_fqcn.clone());
        names.insert(spec.proxy_fqcn.clone());
    }
    for spec in search_results {
        names.insert(spec.source_fqcn.clone());
        names.insert(spec.result_fqcn.clone());
    }
    for spec in proxy_deferred {
        names.insert(spec.target_fqcn.clone());
        names.insert(spec.proxy_fqcn.clone());
    }
    for spec in extension_specs {
        names.insert(spec.source_interface_fqcn.clone());
        names.insert(spec.extension_interface_fqcn.clone());
        names.insert(spec.extension_class_fqcn.clone());
    }

    let mut sorted: Vec<String> = names.into_iter().collect();
    sorted.sort();
    sorted
}

fn build_interception_type_names(
    base_class_map: &FxHashMap<String, ClassInfo>,
    generated_class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
    interceptors: &[di_resolver::InterceptorSpec],
    factories: &[di_resolver::FactorySpec],
    proxies: &[di_resolver::ProxySpec],
    search_results: &[SearchResultsSpec],
    proxy_deferred: &[ProxyDeferredSpec],
    extension_specs: &[ExtensionSpec],
) -> Vec<String> {
    let is_known_type = |name: &str| {
        base_class_map.contains_key(name)
            || generated_class_map.contains_key(name)
            || di_config.virtual_types.contains_key(name)
    };

    let mut names: FxHashSet<String> = build_argument_type_names(
        base_class_map,
        generated_class_map,
        di_config,
        interceptors,
        factories,
        proxies,
        search_results,
        proxy_deferred,
        extension_specs,
    )
    .into_iter()
    .collect();

    names.extend(di_config.plugins.keys().cloned());
    for (from, to) in &di_config.preferences {
        if is_known_type(from) {
            names.insert(from.clone());
        }
        if is_known_type(to) {
            names.insert(to.clone());
        }
    }

    // PHP's interception.php includes the ORIGINAL (non-Interceptor) class name for every
    // intercepted class — marked true — in addition to ClassName\Interceptor.
    // build_argument_type_names excluded intercepted concretes (they only appear as
    // ClassName\Interceptor in arguments), but we must add them back here so that
    // interception.php has full key coverage of the real class universe.
    for spec in interceptors {
        let target = spec.fqcn.trim_start_matches('\\').to_string();
        names.insert(target);
    }

    let mut sorted: Vec<String> = names.into_iter().collect();
    sorted.sort();
    sorted
}

fn build_direct_interception_map(
    interceptors: &[di_resolver::InterceptorSpec],
) -> FxHashMap<String, String> {
    let mut map: FxHashMap<String, String> = interceptors
        .iter()
        .map(|spec| {
            let fqcn = spec.fqcn.trim_start_matches('\\').to_string();
            (fqcn.clone(), format!("{fqcn}\\Interceptor"))
        })
        .collect();

    // setup:di:compile always emits these interception preferences. They are
    // required for setup/compiler metadata parity, including the naming
    // collision case where "...\Code\Generator\Interceptor" is both a real
    // class and the intercepted alias for "...\Code\Generator".
    for fqcn in [
        "Magento\\Framework\\Interception",
        "Magento\\Framework\\Interception\\Code\\Generator",
        "Magento\\Setup\\Module\\Di\\Code\\Generator",
    ] {
        map.insert(fqcn.to_string(), format!("{fqcn}\\Interceptor"));
    }

    map
}

fn build_interception_preference_overrides(
    di_config: &DiConfig,
    direct_interceptors: &FxHashMap<String, String>,
) -> FxHashMap<String, String> {
    // Step 1 (PHP PreferencesResolving): recursively resolve each preference
    // value through the existing preference graph.
    let mut resolved_preferences: FxHashMap<String, String> = FxHashMap::default();
    for (from, to) in &di_config.preferences {
        let from_norm = normalize_fqcn(from);
        if di_config.virtual_types.contains_key(&from_norm) {
            continue;
        }
        let resolved = resolve_preference_recursive(to, &di_config.preferences);
        let substituted = direct_interceptors
            .get(resolved.as_str())
            .cloned()
            .unwrap_or(resolved);
        resolved_preferences.insert(from_norm, substituted);
    }

    // Step 2 (PHP InterceptorSubstitution): merge direct class->interceptor map,
    // then let resolved preferences override those entries.
    let mut merged = direct_interceptors.clone();
    for (from, to) in resolved_preferences {
        merged.insert(from, to);
    }

    // Step 3 (PHP InterceptionPreferencesResolving): run recursive preference
    // resolution once more on the merged map.
    let snapshot = merged.clone();
    for value in merged.values_mut() {
        *value = resolve_preference_recursive(value, &snapshot);
    }

    // Only emit overrides that change/add values compared to raw di.xml prefs.
    let mut overrides = FxHashMap::default();
    for (from, to) in merged {
        match di_config.preferences.get(from.as_str()) {
            Some(existing) if normalize_fqcn(existing) == normalize_fqcn(&to) => {}
            _ => {
                overrides.insert(from, to);
            }
        }
    }

    overrides
}

fn resolve_preference_recursive(start: &str, prefs: &FxHashMap<String, String>) -> String {
    let mut visited = FxHashSet::default();
    let mut current = normalize_fqcn(start);
    loop {
        if !visited.insert(current.clone()) {
            return current;
        }
        let Some(next) = prefs.get(current.as_str()) else {
            return current;
        };
        let next_norm = normalize_fqcn(next);
        if next_norm == current {
            return current;
        }
        current = next_norm;
    }
}

fn normalize_fqcn(value: &str) -> String {
    value.trim().trim_start_matches('\\').to_string()
}

fn build_interception_registry(
    type_names: &[String],
    interceptors: &[di_resolver::InterceptorSpec],
    proxies: &[di_resolver::ProxySpec],
    proxy_deferred: &[ProxyDeferredSpec],
    di_config: &DiConfig,
    class_map: &FxHashMap<String, ClassInfo>,
) -> FxHashMap<String, bool> {
    let mut intercepted_targets: FxHashSet<String> = interceptors
        .iter()
        .map(|spec| spec.fqcn.trim_start_matches('\\').to_string())
        .collect();

    // Active plugin owners are intercepted even when no InterceptorSpec file is emitted
    // (e.g. abstract classes/interfaces used only as plugin declaration owners).
    // Normalize the owner name to strip any leading backslash so it matches type_names.
    intercepted_targets.extend(di_config.plugins.iter().filter_map(|(owner, plugins)| {
        if plugins.iter().any(|plugin| !plugin.disabled) {
            Some(owner.trim_start_matches('\\').to_string())
        } else {
            None
        }
    }));

    // Virtual types whose target is intercepted are intercepted too. Resolve
    // transitively to cover virtual type chains.
    let mut changed = true;
    while changed {
        changed = false;
        for (vt_name, vt) in &di_config.virtual_types {
            let direct = vt.type_name.trim_start_matches('\\');
            if intercepted_targets.contains(direct) && intercepted_targets.insert(vt_name.clone()) {
                changed = true;
            }
        }
    }

    // Proxy classes of intercepted targets are also marked intercepted.
    // PHP marks SomeClass\Proxy as true when SomeClass is intercepted because
    // instantiating the proxy would invoke the interceptor chain.
    for spec in proxies {
        let target = spec.target_fqcn.trim_start_matches('\\');
        if intercepted_targets.contains(target) {
            intercepted_targets.insert(spec.proxy_fqcn.trim_start_matches('\\').to_string());
        }
    }
    for spec in proxy_deferred {
        let target = spec.target_fqcn.trim_start_matches('\\');
        if intercepted_targets.contains(target) {
            intercepted_targets.insert(spec.proxy_fqcn.trim_start_matches('\\').to_string());
        }
    }

    // Propagate intercepted status to all classes that implement an intercepted interface.
    // PHP marks a class as intercepted whenever any interface it implements (transitively)
    // has plugins — e.g. Magento\Framework\App\ActionInterface with CustomerNotification
    // propagates to Magento\Backend\App\Action (which implements ActionInterface).
    //
    // Build a reverse-index (interface → implementors) once, then BFS from each newly
    // intercepted seed. O(n + edges) single pass vs. the old O(n × depth) fixed-point loop.
    let mut implementors: FxHashMap<&str, Vec<&str>> = FxHashMap::default();
    for (fqcn, info) in class_map.iter() {
        for iface in &info.implements {
            implementors
                .entry(iface.trim_start_matches('\\'))
                .or_default()
                .push(fqcn.as_str());
        }
    }
    let mut queue: std::collections::VecDeque<String> =
        intercepted_targets.iter().cloned().collect();
    while let Some(intercepted) = queue.pop_front() {
        for &implementor in implementors.get(intercepted.as_str()).into_iter().flatten() {
            if intercepted_targets.insert(implementor.to_string()) {
                queue.push_back(implementor.to_string());
            }
        }
    }

    // Propagate intercepted status up the class inheritance chain.
    // PHP marks abstract/concrete ancestors as intercepted when a descendant
    // class is intercepted (e.g. Magento\Backend\App\Action).
    let intercepted_snapshot: Vec<String> = intercepted_targets.iter().cloned().collect();
    for fqcn in intercepted_snapshot {
        let mut cursor = class_map
            .get(&fqcn)
            .and_then(|info| info.extends.as_ref())
            .map(|s| s.trim_start_matches('\\').to_string());
        while let Some(parent) = cursor {
            let next = class_map.get(&parent).and_then(|info| info.extends.clone());
            intercepted_targets.insert(parent.clone());
            cursor = next;
        }
    }

    type_names
        .iter()
        .map(|name| {
            let intercepted = intercepted_targets.contains(name);
            (name.clone(), intercepted)
        })
        .collect()
}

fn print_summary(
    interceptors: &[di_resolver::InterceptorSpec],
    factories: &[di_resolver::FactorySpec],
    proxies: &[di_resolver::ProxySpec],
    args_map: &FxHashMap<String, Vec<di_resolver::ResolvedArg>>,
    all_fqcns: &FxHashMap<String, bool>,
) {
    println!("Dry run summary:");
    println!("  Interceptors:   {}", interceptors.len());
    println!("  Factories:      {}", factories.len());
    println!("  Proxies:        {}", proxies.len());
    println!("  Classes with resolved args: {}", args_map.len());
    println!("  Total FQCNs (for interception.php): {}", all_fqcns.len());
}

#[derive(Debug, Serialize)]
struct ArchiveCompareSummary {
    code_missing: usize,
    code_extra: usize,
    code_changed: usize,
    metadata_missing: usize,
    metadata_extra: usize,
    metadata_changed: usize,
}

impl ArchiveCompareSummary {
    fn is_clean(&self) -> bool {
        self.code_missing == 0
            && self.code_extra == 0
            && self.code_changed == 0
            && self.metadata_missing == 0
            && self.metadata_extra == 0
            && self.metadata_changed == 0
    }
}

#[derive(Debug)]
struct RelativeDiff {
    missing: Vec<String>,
    extra: Vec<String>,
    changed: Vec<String>,
}

fn compare_against_archive(
    output_root: &Path,
    archive_root: &Path,
    report_dir: &Path,
    php_bin: &str,
) -> std::io::Result<ArchiveCompareSummary> {
    let archive_code = archive_root.join("_code");
    let archive_metadata = archive_root.join("_metadata");
    if !archive_code.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("archive code dir not found: {}", archive_code.display()),
        ));
    }
    if !archive_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "archive metadata dir not found: {}",
                archive_metadata.display()
            ),
        ));
    }

    let output_code = output_root.join("code");
    let output_metadata = output_root.join("metadata");

    let code_diff = diff_relative_files(&archive_code, &output_code)?;
    let metadata_diff = diff_relative_files(&archive_metadata, &output_metadata)?;

    std::fs::create_dir_all(report_dir)?;
    write_diff_list(&report_dir.join("code.missing.txt"), &code_diff.missing)?;
    write_diff_list(&report_dir.join("code.extra.txt"), &code_diff.extra)?;
    write_diff_list(&report_dir.join("code.changed.txt"), &code_diff.changed)?;
    write_diff_list(
        &report_dir.join("metadata.missing.txt"),
        &metadata_diff.missing,
    )?;
    write_diff_list(&report_dir.join("metadata.extra.txt"), &metadata_diff.extra)?;
    write_diff_list(
        &report_dir.join("metadata.changed.txt"),
        &metadata_diff.changed,
    )?;
    write_comparable_metadata_reports(&archive_metadata, &output_metadata, report_dir, php_bin)?;

    let summary = ArchiveCompareSummary {
        code_missing: code_diff.missing.len(),
        code_extra: code_diff.extra.len(),
        code_changed: code_diff.changed.len(),
        metadata_missing: metadata_diff.missing.len(),
        metadata_extra: metadata_diff.extra.len(),
        metadata_changed: metadata_diff.changed.len(),
    };
    let summary_json = serde_json::to_string_pretty(&summary).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(report_dir.join("summary.json"), summary_json)?;

    Ok(summary)
}

fn write_comparable_metadata_reports(
    archive_metadata_dir: &Path,
    output_metadata_dir: &Path,
    report_dir: &Path,
    php_bin: &str,
) -> std::io::Result<()> {
    let archive_files = collect_relative_files(archive_metadata_dir)?;
    let output_files = collect_relative_files(output_metadata_dir)?;
    let mut common: Vec<String> = archive_files.intersection(&output_files).cloned().collect();
    common.sort();

    let comparable_dir = report_dir.join("comparable_metadata");
    std::fs::create_dir_all(&comparable_dir)?;

    // Parallelize: each file requires two `php -r` subprocess spawns (archive + output).
    // Sequential execution was responsible for ~16s of archive-compare runtime; par_iter
    // reduces this to ceil(N/threads) × per-file cost.
    let manifest_lines: Vec<std::io::Result<String>> = common
        .par_iter()
        .map(|rel| {
            let archive_src = archive_metadata_dir.join(rel);
            let output_src = output_metadata_dir.join(rel);
            let stem = comparable_metadata_stem(rel);
            let archive_dst = comparable_dir.join(format!("{stem}.archive.json"));
            let output_dst = comparable_dir.join(format!("{stem}.output.json"));
            let archive_json = normalize_metadata_to_json_bytes(&archive_src, php_bin)?;
            let output_json = normalize_metadata_to_json_bytes(&output_src, php_bin)?;
            std::fs::write(&archive_dst, &archive_json)?;
            std::fs::write(&output_dst, &output_json)?;

            let report = build_comparable_metadata_report(rel, &archive_json, &output_json)?;
            let report_json = serde_json::to_string_pretty(&report)
                .unwrap_or_else(|_| "{\"error\":\"serialize\"}".to_string());
            let report_dst = comparable_dir.join(format!("{stem}_report.json"));
            std::fs::write(&report_dst, report_json)?;
            let text_report = render_comparable_metadata_report_text(&report);
            let text_report_dst = comparable_dir.join(format!("{stem}_report.txt"));
            std::fs::write(&text_report_dst, text_report)?;

            Ok(format!(
                "{rel}\t{stem}\t{stem}.archive.json\t{stem}.output.json\t{stem}_report.json\t{stem}_report.txt\n"
            ))
        })
        .collect();

    // Preserve original sorted order in the manifest; collect errors.
    let mut manifest = String::new();
    for line in manifest_lines {
        manifest.push_str(&line?);
    }
    std::fs::write(comparable_dir.join("manifest.txt"), manifest)?;

    Ok(())
}

fn comparable_metadata_stem(rel: &str) -> String {
    let safe = rel.replace(['/', '\\'], "__");
    format!("comparable_{safe}")
}

fn normalize_metadata_to_json_bytes(src: &Path, php_bin: &str) -> std::io::Result<Vec<u8>> {
    let script = r#"
$file = $argv[1] ?? '';
if ($file === '' || !is_file($file)) {
    fwrite(STDERR, "missing metadata file\n");
    exit(2);
}
$data = include $file;
$normalize = function ($value) use (&$normalize) {
    if (!is_array($value)) {
        return $value;
    }
    if (array_is_list($value)) {
        $out = [];
        foreach ($value as $item) {
            $out[] = $normalize($item);
        }
        return $out;
    }
    $keys = array_keys($value);
    usort($keys, function ($a, $b) {
        return strcmp((string)$a, (string)$b);
    });
    $out = [];
    foreach ($keys as $key) {
        $out[$key] = $normalize($value[$key]);
    }
    return $out;
};
$normalized = $normalize($data);
$json = json_encode(
    $normalized,
    JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE
);
if ($json === false) {
    fwrite(STDERR, "json_encode failed\n");
    exit(3);
}
echo $json, "\n";
"#;

    let output = Command::new(php_bin)
        .arg("-r")
        .arg(script)
        .arg(src.as_os_str())
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "failed to normalize metadata {}: {}",
                src.display(),
                stderr.trim()
            ),
        ));
    }
    Ok(output.stdout)
}

#[derive(Debug, Default, Serialize)]
struct ComparableMetadataReport {
    file: String,
    summary: ComparableReportSummary,
    sections: BTreeMap<String, ComparableReportSummary>,
    type_mismatches_by_pair: BTreeMap<String, usize>,
    high_risk_mismatches_sample: Vec<ComparableTypeMismatchSample>,
    missing_paths_sample: Vec<String>,
    extra_paths_sample: Vec<String>,
    value_mismatches_sample: Vec<ComparableValueMismatchSample>,
    severity_score: u64,
}

#[derive(Debug, Default, Clone, Serialize)]
struct ComparableReportSummary {
    missing_paths: usize,
    extra_paths: usize,
    type_mismatches: usize,
    value_mismatches: usize,
    high_risk_mismatches: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ComparableTypeMismatchSample {
    path: String,
    truth_type: String,
    output_type: String,
    pair: String,
}

#[derive(Debug, Clone, Serialize)]
struct ComparableValueMismatchSample {
    path: String,
    truth: String,
    output: String,
}

#[derive(Default)]
struct ComparableReportAccumulator {
    summary: ComparableReportSummary,
    sections: FxHashMap<String, ComparableReportSummary>,
    type_mismatches_by_pair: FxHashMap<String, usize>,
    high_risk_mismatches_sample: Vec<ComparableTypeMismatchSample>,
    missing_paths_sample: Vec<String>,
    extra_paths_sample: Vec<String>,
    value_mismatches_sample: Vec<ComparableValueMismatchSample>,
}

fn build_comparable_metadata_report(
    file: &str,
    archive_json: &[u8],
    output_json: &[u8],
) -> std::io::Result<ComparableMetadataReport> {
    let truth: serde_json::Value = serde_json::from_slice(archive_json).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("failed to parse archive comparable json for {}: {e}", file),
        )
    })?;
    let ours: serde_json::Value = serde_json::from_slice(output_json).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("failed to parse output comparable json for {}: {e}", file),
        )
    })?;

    let mut acc = ComparableReportAccumulator::default();
    compare_json_values(&truth, &ours, "", &mut acc);
    let severity_score = (acc.summary.high_risk_mismatches as u64) * 100
        + (acc.summary.type_mismatches as u64) * 10
        + (acc.summary.value_mismatches as u64)
        + (acc.summary.missing_paths as u64)
        + (acc.summary.extra_paths as u64);

    let mut sections = BTreeMap::new();
    for (k, v) in acc.sections {
        sections.insert(k, v);
    }
    let mut type_mismatches_by_pair = BTreeMap::new();
    for (k, v) in acc.type_mismatches_by_pair {
        type_mismatches_by_pair.insert(k, v);
    }

    Ok(ComparableMetadataReport {
        file: file.to_string(),
        summary: acc.summary,
        sections,
        type_mismatches_by_pair,
        high_risk_mismatches_sample: acc.high_risk_mismatches_sample,
        missing_paths_sample: acc.missing_paths_sample,
        extra_paths_sample: acc.extra_paths_sample,
        value_mismatches_sample: acc.value_mismatches_sample,
        severity_score,
    })
}

fn compare_json_values(
    truth: &serde_json::Value,
    ours: &serde_json::Value,
    path: &str,
    acc: &mut ComparableReportAccumulator,
) {
    const SAMPLE_LIMIT: usize = 200;

    let truth_ty = json_type_name(truth);
    let ours_ty = json_type_name(ours);
    if truth_ty != ours_ty {
        let pair = format!("{truth_ty}|{ours_ty}");
        acc.summary.type_mismatches += 1;
        increment_section(acc, path, |s| s.type_mismatches += 1);
        *acc.type_mismatches_by_pair.entry(pair.clone()).or_insert(0) += 1;
        let high_risk = is_high_risk_pair(truth_ty, ours_ty);
        if high_risk {
            acc.summary.high_risk_mismatches += 1;
            increment_section(acc, path, |s| s.high_risk_mismatches += 1);
            if acc.high_risk_mismatches_sample.len() < SAMPLE_LIMIT {
                acc.high_risk_mismatches_sample
                    .push(ComparableTypeMismatchSample {
                        path: path.to_string(),
                        truth_type: truth_ty.to_string(),
                        output_type: ours_ty.to_string(),
                        pair,
                    });
            }
        }
        return;
    }

    match (truth, ours) {
        (serde_json::Value::Object(t), serde_json::Value::Object(o)) => {
            for (k, tv) in t {
                let child_path = join_object_path(path, k);
                if let Some(ov) = o.get(k) {
                    compare_json_values(tv, ov, &child_path, acc);
                } else {
                    acc.summary.missing_paths += 1;
                    increment_section(acc, &child_path, |s| s.missing_paths += 1);
                    if acc.missing_paths_sample.len() < SAMPLE_LIMIT {
                        acc.missing_paths_sample.push(child_path);
                    }
                }
            }
            for k in o.keys() {
                if !t.contains_key(k) {
                    let child_path = join_object_path(path, k);
                    acc.summary.extra_paths += 1;
                    increment_section(acc, &child_path, |s| s.extra_paths += 1);
                    if acc.extra_paths_sample.len() < SAMPLE_LIMIT {
                        acc.extra_paths_sample.push(child_path);
                    }
                }
            }
        }
        (serde_json::Value::Array(t), serde_json::Value::Array(o)) => {
            let shared = std::cmp::min(t.len(), o.len());
            for idx in 0..shared {
                let child_path = join_array_path(path, idx);
                compare_json_values(&t[idx], &o[idx], &child_path, acc);
            }
            for idx in shared..t.len() {
                let child_path = join_array_path(path, idx);
                acc.summary.missing_paths += 1;
                increment_section(acc, &child_path, |s| s.missing_paths += 1);
                if acc.missing_paths_sample.len() < SAMPLE_LIMIT {
                    acc.missing_paths_sample.push(child_path);
                }
            }
            for idx in shared..o.len() {
                let child_path = join_array_path(path, idx);
                acc.summary.extra_paths += 1;
                increment_section(acc, &child_path, |s| s.extra_paths += 1);
                if acc.extra_paths_sample.len() < SAMPLE_LIMIT {
                    acc.extra_paths_sample.push(child_path);
                }
            }
        }
        _ => {
            if truth != ours {
                acc.summary.value_mismatches += 1;
                increment_section(acc, path, |s| s.value_mismatches += 1);
                if acc.value_mismatches_sample.len() < SAMPLE_LIMIT {
                    acc.value_mismatches_sample
                        .push(ComparableValueMismatchSample {
                            path: path.to_string(),
                            truth: compact_json_value(truth),
                            output: compact_json_value(ours),
                        });
                }
            }
        }
    }
}

fn increment_section<F>(acc: &mut ComparableReportAccumulator, path: &str, f: F)
where
    F: FnOnce(&mut ComparableReportSummary),
{
    let key = section_from_path(path);
    let entry = acc.sections.entry(key).or_default();
    f(entry);
}

fn section_from_path(path: &str) -> String {
    if path.is_empty() {
        return "root".to_string();
    }
    if let Some(rest) = path.strip_prefix('[') {
        return if rest.is_empty() {
            "list".to_string()
        } else {
            "list".to_string()
        };
    }
    let first = path.split('.').next().unwrap_or("root");
    let first = first.split('[').next().unwrap_or(first);
    if first.is_empty() {
        "root".to_string()
    } else {
        first.to_string()
    }
}

fn join_object_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn join_array_path(parent: &str, index: usize) -> String {
    if parent.is_empty() {
        format!("[{index}]")
    } else {
        format!("{parent}[{index}]")
    }
}

fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "NULL",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn is_high_risk_pair(truth_ty: &str, ours_ty: &str) -> bool {
    let truth_container = truth_ty == "array" || truth_ty == "object";
    let ours_container = ours_ty == "array" || ours_ty == "object";
    if truth_container != ours_container {
        return true;
    }
    matches!(
        (truth_ty, ours_ty),
        ("NULL", "array") | ("array", "NULL") | ("NULL", "object") | ("object", "NULL")
    )
}

fn compact_json_value(v: &serde_json::Value) -> String {
    const LIMIT: usize = 220;
    let raw = serde_json::to_string(v).unwrap_or_else(|_| "<unserializable>".to_string());
    if raw.len() <= LIMIT {
        raw
    } else {
        format!("{}...", &raw[..LIMIT])
    }
}

fn render_comparable_metadata_report_text(report: &ComparableMetadataReport) -> String {
    const TOP_N: usize = 10;
    const SAMPLE_N: usize = 20;

    let mut out = String::new();
    out.push_str(&format!("file: {}\n", report.file));
    out.push_str(&format!("severity_score: {}\n", report.severity_score));
    out.push_str("summary:\n");
    out.push_str(&format!(
        "  missing_paths: {}\n",
        report.summary.missing_paths
    ));
    out.push_str(&format!("  extra_paths: {}\n", report.summary.extra_paths));
    out.push_str(&format!(
        "  type_mismatches: {}\n",
        report.summary.type_mismatches
    ));
    out.push_str(&format!(
        "  value_mismatches: {}\n",
        report.summary.value_mismatches
    ));
    out.push_str(&format!(
        "  high_risk_mismatches: {}\n",
        report.summary.high_risk_mismatches
    ));

    let mut section_rows: Vec<(&String, &ComparableReportSummary)> =
        report.sections.iter().collect();
    section_rows.sort_by(|(ka, va), (kb, vb)| {
        let sa = section_weight(va);
        let sb = section_weight(vb);
        sb.cmp(&sa).then_with(|| ka.cmp(kb))
    });
    out.push_str("top_sections:\n");
    for (section, stats) in section_rows.into_iter().take(TOP_N) {
        out.push_str(&format!(
            "  - {}: missing={}, extra={}, type={}, value={}, high_risk={}\n",
            section,
            stats.missing_paths,
            stats.extra_paths,
            stats.type_mismatches,
            stats.value_mismatches,
            stats.high_risk_mismatches
        ));
    }

    let mut pair_rows: Vec<(&String, &usize)> = report.type_mismatches_by_pair.iter().collect();
    pair_rows.sort_by(|(ka, va), (kb, vb)| vb.cmp(va).then_with(|| ka.cmp(kb)));
    out.push_str("top_type_pairs:\n");
    for (pair, count) in pair_rows.into_iter().take(TOP_N) {
        out.push_str(&format!("  - {}: {}\n", pair, count));
    }

    let suggestions = infer_comparable_fix_categories(report);
    out.push_str("suggested_fix_categories:\n");
    if suggestions.is_empty() {
        out.push_str("  - none\n");
    } else {
        for suggestion in suggestions {
            out.push_str(&format!("  - {}\n", suggestion));
        }
    }

    out.push_str("high_risk_samples:\n");
    for sample in report.high_risk_mismatches_sample.iter().take(SAMPLE_N) {
        out.push_str(&format!(
            "  - {} [{} -> {}]\n",
            sample.path, sample.truth_type, sample.output_type
        ));
    }
    out.push_str("missing_paths_sample:\n");
    for path in report.missing_paths_sample.iter().take(SAMPLE_N) {
        out.push_str(&format!("  - {}\n", path));
    }
    out.push_str("extra_paths_sample:\n");
    for path in report.extra_paths_sample.iter().take(SAMPLE_N) {
        out.push_str(&format!("  - {}\n", path));
    }
    out.push_str("value_mismatches_sample:\n");
    for sample in report.value_mismatches_sample.iter().take(SAMPLE_N) {
        out.push_str(&format!(
            "  - {} [truth={}, output={}]\n",
            sample.path, sample.truth, sample.output
        ));
    }

    out
}

fn section_weight(summary: &ComparableReportSummary) -> u64 {
    (summary.high_risk_mismatches as u64) * 100
        + (summary.type_mismatches as u64) * 10
        + (summary.value_mismatches as u64)
        + (summary.missing_paths as u64)
        + (summary.extra_paths as u64)
}

fn infer_comparable_fix_categories(report: &ComparableMetadataReport) -> Vec<String> {
    let mut categories: BTreeSet<String> = BTreeSet::new();
    if report
        .missing_paths_sample
        .iter()
        .chain(report.extra_paths_sample.iter())
        .any(|path| path.starts_with("arguments."))
    {
        categories.insert("argument merge/resolution parity (arguments.* drift)".to_string());
    }
    if report
        .missing_paths_sample
        .iter()
        .chain(report.extra_paths_sample.iter())
        .any(|path| path.starts_with("instanceTypes."))
    {
        categories.insert(
            "instanceTypes mapping parity (virtual/interceptor target resolution)".to_string(),
        );
    }
    if report
        .missing_paths_sample
        .iter()
        .chain(report.extra_paths_sample.iter())
        .any(|path| path.starts_with("preferences."))
    {
        categories.insert("preferences parity (owner -> resolved type alignment)".to_string());
    }
    let has_string_number = report
        .type_mismatches_by_pair
        .keys()
        .any(|pair| pair == "string|number" || pair == "number|string");
    if has_string_number {
        categories
            .insert("scalar normalization parity (numeric string vs numeric value)".to_string());
    }
    let has_container_shape = report
        .type_mismatches_by_pair
        .keys()
        .any(|pair| pair == "object|array" || pair == "array|object");
    if has_container_shape {
        categories.insert(
            "array/object shape parity (list vs associative map normalization)".to_string(),
        );
    }
    let has_null_container = report.type_mismatches_by_pair.keys().any(|pair| {
        pair == "NULL|object"
            || pair == "object|NULL"
            || pair == "NULL|array"
            || pair == "array|NULL"
    });
    if has_null_container {
        categories.insert(
            "null-container mismatch parity (missing node vs explicit null/collection)".to_string(),
        );
    }
    if report.file.contains("plugin-list.php") {
        categories.insert(
            "plugin list key/surface parity (method key expansion and owner coverage)".to_string(),
        );
    }
    if report.file == "interception.php" {
        categories.insert(
            "interception registry parity (interceptor set and plugin owner aggregation)"
                .to_string(),
        );
    }
    categories.into_iter().collect()
}

fn diff_relative_files(archive_dir: &Path, output_dir: &Path) -> std::io::Result<RelativeDiff> {
    let archive_files = collect_relative_files(archive_dir)?;
    let output_files = collect_relative_files(output_dir)?;

    let mut missing: Vec<String> = archive_files.difference(&output_files).cloned().collect();
    let mut extra: Vec<String> = output_files.difference(&archive_files).cloned().collect();
    let mut changed = Vec::new();

    for rel in archive_files.intersection(&output_files) {
        let archive_path = archive_dir.join(rel);
        let output_path = output_dir.join(rel);
        if files_differ(&archive_path, &output_path)? {
            changed.push(rel.clone());
        }
    }

    missing.sort();
    extra.sort();
    changed.sort();

    Ok(RelativeDiff {
        missing,
        extra,
        changed,
    })
}

fn collect_relative_files(root: &Path) -> std::io::Result<FxHashSet<String>> {
    let mut out = FxHashSet::default();
    if !root.exists() {
        return Ok(out);
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel);
        }
    }

    Ok(out)
}

fn write_diff_list(path: &Path, lines: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut content = String::new();
    for line in lines {
        content.push_str(line);
        content.push('\n');
    }
    std::fs::write(path, content)
}

fn files_differ(left: &Path, right: &Path) -> std::io::Result<bool> {
    let left_meta = std::fs::metadata(left)?;
    let right_meta = std::fs::metadata(right)?;
    if left_meta.len() != right_meta.len() {
        return Ok(true);
    }
    let left_bytes = std::fs::read(left)?;
    let right_bytes = std::fs::read(right)?;
    Ok(left_bytes != right_bytes)
}

fn synthetic_factory_class_info(factory_fqcn: &str) -> ClassInfo {
    let normalized = factory_fqcn.trim_start_matches('\\');
    let target_fqcn = normalized
        .strip_suffix("Factory")
        .unwrap_or(normalized)
        .to_string();
    let escaped_target = target_fqcn.replace('\\', "\\\\");
    let (namespace, name) = if let Some((ns, class_name)) = normalized.rsplit_once('\\') {
        (ns.to_string(), class_name.to_string())
    } else {
        (String::new(), normalized.to_string())
    };

    ClassInfo {
        path: PathBuf::from("__generated__/factory.php"),
        namespace,
        name,
        fqcn: normalized.to_string(),
        kind: ClassKind::Class,
        extends: None,
        implements: vec![],
        constructor: Some(Constructor {
            params: vec![
                ConstructorParam {
                    name: "objectManager".to_string(),
                    type_hint: Some("Magento\\Framework\\ObjectManagerInterface".to_string()),
                    is_optional: false,
                    default_value: None,
                    is_primitive: false,
                    is_variadic: false,
                    is_promoted: false,
                },
                ConstructorParam {
                    name: "instanceName".to_string(),
                    type_hint: None,
                    is_optional: true,
                    default_value: Some(format!("'\\\\{}'", escaped_target)),
                    is_primitive: false,
                    is_variadic: false,
                    is_promoted: false,
                },
            ],
        }),
        is_abstract: false,
        is_final: false,
        public_methods: vec![MethodSignature {
            name: "create".to_string(),
            params: vec![MethodParam {
                name: "data".to_string(),
                type_hint: Some("array".to_string()),
                has_default: true,
                default_value: Some("[]".to_string()),
                is_variadic: false,
                is_by_ref: false,
            }],
            return_type: None,
            is_static: false,
            returns_reference: false,
        }],
    }
}

#[derive(Clone, Debug)]
struct SearchResultsSpec {
    result_fqcn: String,
    source_fqcn: String,
}

#[derive(Clone, Debug)]
struct ProxyDeferredSpec {
    proxy_fqcn: String,
    target_fqcn: String,
}

fn detect_search_results_specs(
    class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
    factories: &[FactorySpec],
    composer_index: Option<&ComposerAutoloadIndex>,
) -> Vec<SearchResultsSpec> {
    let mut specs = Vec::new();
    let mut seen = FxHashSet::default();

    let mut emit = |result_fqcn: String| {
        let normalized = result_fqcn.trim().trim_start_matches('\\').to_string();
        if !normalized.ends_with("SearchResults") {
            return;
        }
        if class_map.contains_key(&normalized) || !seen.insert(normalized.clone()) {
            return;
        }
        let Some(source_fqcn) = normalized.strip_suffix("SearchResults") else {
            return;
        };
        let source_fqcn = source_fqcn.to_string();
        if source_fqcn.is_empty() {
            return;
        }
        if !class_exists_in_scan_or_composer(class_map, composer_index, &source_fqcn) {
            return;
        }
        specs.push(SearchResultsSpec {
            result_fqcn: normalized,
            source_fqcn,
        });
    };

    for spec in factories {
        emit(spec.target_fqcn.clone());
    }
    for (for_type, to_type) in &di_config.preferences {
        if for_type.ends_with("SearchResultsInterface") {
            emit(to_type.clone());
        }
    }

    specs.sort_by(|a, b| a.result_fqcn.cmp(&b.result_fqcn));
    specs
}

fn detect_proxy_deferred_specs(
    class_map: &FxHashMap<String, ClassInfo>,
    factories: &[FactorySpec],
    composer_index: Option<&ComposerAutoloadIndex>,
) -> Vec<ProxyDeferredSpec> {
    let mut specs = Vec::new();
    let mut seen = FxHashSet::default();

    for spec in factories {
        let normalized = spec.target_fqcn.trim().trim_start_matches('\\').to_string();
        let Some(target_fqcn) = normalized.strip_suffix("\\ProxyDeferred") else {
            continue;
        };
        let target_fqcn = target_fqcn.to_string();
        if target_fqcn.is_empty() {
            continue;
        }
        if class_map.contains_key(&normalized) || !seen.insert(normalized.clone()) {
            continue;
        }
        if !class_exists_in_scan_or_composer(class_map, composer_index, &target_fqcn) {
            continue;
        }
        specs.push(ProxyDeferredSpec {
            proxy_fqcn: normalized,
            target_fqcn,
        });
    }

    specs.sort_by(|a, b| a.proxy_fqcn.cmp(&b.proxy_fqcn));
    specs
}

fn class_exists_in_scan_or_composer(
    class_map: &FxHashMap<String, ClassInfo>,
    composer_index: Option<&ComposerAutoloadIndex>,
    fqcn: &str,
) -> bool {
    let normalized = fqcn.trim().trim_start_matches('\\');
    if class_map.contains_key(normalized) {
        return true;
    }
    composer_index
        .and_then(|index| index.resolve_class_path(normalized))
        .is_some()
}

fn merge_plugins_for_interception(
    global_configs: &[DiConfig],
    area_configs: &[DiConfig],
) -> FxHashMap<String, Vec<Plugin>> {
    let mut merged: FxHashMap<String, FxHashMap<String, Plugin>> = FxHashMap::default();

    // Phase 1: global di.xml files — later wins (including disabled overrides).
    for cfg in global_configs {
        for (owner, plugins) in &cfg.plugins {
            let owner_plugins = merged.entry(owner.clone()).or_default();
            for plugin in plugins {
                match owner_plugins.get_mut(&plugin.name) {
                    None => {
                        owner_plugins.insert(plugin.name.clone(), plugin.clone());
                    }
                    Some(existing) => {
                        if plugin.type_name.is_empty() {
                            // Disabled-only override: preserve type_name, update disabled flag.
                            existing.disabled = plugin.disabled;
                            if plugin.sort_order != 0 {
                                existing.sort_order = plugin.sort_order;
                            }
                        } else {
                            let was_disabled = existing.disabled;
                            *existing = plugin.clone();
                            // PHP sticky-disabled: once disabled, stays disabled.
                            if was_disabled {
                                existing.disabled = true;
                            }
                        }
                    }
                }
            }
        }
    }

    // Phase 2: area-specific di.xml files — active plugins can add/re-enable; disables ignored.
    for cfg in area_configs {
        for (owner, plugins) in &cfg.plugins {
            let owner_plugins = merged.entry(owner.clone()).or_default();
            for plugin in plugins {
                if plugin.disabled {
                    // Area-specific disables do not remove globally active plugins.
                    continue;
                }
                match owner_plugins.get_mut(&plugin.name) {
                    None => {
                        owner_plugins.insert(plugin.name.clone(), plugin.clone());
                    }
                    Some(existing) => {
                        if existing.disabled {
                            // Plugin disabled globally but active in this area — re-enable.
                            *existing = plugin.clone();
                        }
                    }
                }
            }
        }
    }

    let mut out = FxHashMap::default();
    for (owner, by_name) in merged {
        let mut plugins: Vec<Plugin> = by_name.into_values().collect();
        plugins.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
        out.insert(owner, plugins);
    }
    out
}

fn augment_with_composer_plugin_owner_classes(
    class_map: &mut FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
    composer_index: Option<&ComposerAutoloadIndex>,
) {
    let Some(index) = composer_index else {
        return;
    };

    let mut candidates: FxHashSet<String> = FxHashSet::default();
    for owner in di_config.plugins.keys() {
        let owner = owner.trim_start_matches('\\').to_string();
        if !class_map.contains_key(&owner) {
            candidates.insert(owner.clone());
        }
        let resolved = di_config.get_instance_type(&owner);
        if !class_map.contains_key(&resolved) {
            candidates.insert(resolved);
        }
    }
    for plugins in di_config.plugins.values() {
        for plugin in plugins {
            let plugin_type = plugin.type_name.trim_start_matches('\\').to_string();
            if !class_map.contains_key(&plugin_type) {
                candidates.insert(plugin_type.clone());
            }
            let resolved_plugin_type = di_config.get_instance_type(&plugin_type);
            if !class_map.contains_key(&resolved_plugin_type) {
                candidates.insert(resolved_plugin_type);
            }
        }
    }

    for fqcn in candidates {
        if class_map.contains_key(&fqcn) {
            continue;
        }
        let Some(path) = index.resolve_class_path(&fqcn) else {
            continue;
        };
        if let ExtractResult::Ok(info) = extract_file(&path) {
            class_map.insert(info.fqcn.clone(), info);
        }
    }
}

fn interceptor_target_info_with_inherited_constructor(
    fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) -> Option<ClassInfo> {
    let normalized = fqcn.trim_start_matches('\\');
    let mut info = class_map.get(normalized)?.clone();
    if info.constructor.is_some() {
        return Some(info);
    }

    let mut seen: FxHashSet<String> = FxHashSet::default();
    let mut cursor = info.extends.clone();
    while let Some(parent) = cursor {
        if !seen.insert(parent.clone()) {
            break;
        }
        let Some(parent_info) = class_map.get(&parent) else {
            break;
        };
        if parent_info.constructor.is_some() {
            info.constructor = parent_info.constructor.clone();
            break;
        }
        cursor = parent_info.extends.clone();
    }

    Some(info)
}

fn target_info_with_inherited_public_methods(
    fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) -> Option<ClassInfo> {
    let normalized = fqcn.trim_start_matches('\\');
    let mut info = class_map.get(normalized)?.clone();
    info.public_methods = collect_public_methods_with_inheritance(normalized, class_map);
    Some(info)
}

fn collect_public_methods_with_inheritance(
    fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) -> Vec<MethodSignature> {
    let mut methods = Vec::new();
    let mut seen_names: FxHashSet<String> = FxHashSet::default();
    let mut seen_types: FxHashSet<String> = FxHashSet::default();
    let mut stack = vec![fqcn.to_string()];

    while let Some(current) = stack.pop() {
        if !seen_types.insert(current.clone()) {
            continue;
        }
        let Some(info) = class_map.get(&current) else {
            continue;
        };

        for method in &info.public_methods {
            if seen_names.insert(method.name.clone()) {
                methods.push(method.clone());
            }
        }

        if let Some(parent) = &info.extends {
            stack.push(parent.clone());
        }
        for interface in &info.implements {
            stack.push(interface.clone());
        }
    }

    methods
}

fn enrich_interceptor_specs_with_reflection(
    specs: &mut [di_resolver::InterceptorSpec],
    class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
    magento_root: &Path,
    php_bin: &str,
) {
    // -----------------------------------------------------------------
    // Phase A: collect unique plugin class FQCNs not in class_map,
    // then reflect them all in parallel to build a plugin-method lookup
    // table.  Replaces the old sequential plugin_method_cache loop.
    // -----------------------------------------------------------------
    let plugin_fqcns_to_reflect: FxHashSet<String> = specs
        .iter()
        .flat_map(|spec| {
            spec.plugins.iter().flat_map(|p| {
                let resolved = di_config
                    .get_instance_type(&p.type_name)
                    .trim_start_matches('\\')
                    .to_string();
                let raw = p.type_name.trim_start_matches('\\').to_string();
                [resolved, raw]
            })
        })
        .filter(|fqcn| !fqcn.is_empty() && !class_map.contains_key(fqcn))
        .collect();

    let plugin_method_map: FxHashMap<String, FxHashSet<String>> = plugin_fqcns_to_reflect
        .par_iter()
        .filter_map(|fqcn| {
            let methods = reflect_interceptable_methods(fqcn, magento_root, php_bin)?;
            let names: FxHashSet<String> = methods
                .iter()
                .filter_map(|m| plugin_method_to_intercepted(&m.name))
                .collect();
            if names.is_empty() {
                None
            } else {
                Some((fqcn.clone(), names))
            }
        })
        .collect();

    // -----------------------------------------------------------------
    // Phase B: determine which spec FQCNs need their own reflection,
    // then reflect them all in parallel.  Replaces the old sequential
    // reflection_cache loop.
    // -----------------------------------------------------------------
    let specs_needing_reflection: FxHashSet<String> = specs
        .iter()
        .filter(|spec| spec_needs_reflection(spec, class_map, di_config, &plugin_method_map))
        .map(|spec| spec.fqcn.clone())
        .collect();

    let spec_reflection_map: FxHashMap<String, Vec<MethodSignature>> = specs_needing_reflection
        .par_iter()
        .filter_map(|fqcn| {
            let mut methods = reflect_interceptable_methods(fqcn, magento_root, php_bin)?;
            for m in &mut methods {
                normalize_reflected_method_signature(m, fqcn, class_map);
            }
            Some((fqcn.clone(), methods))
        })
        .collect();

    log::debug!(
        "enrich_interceptor_specs: {} plugin FQCNs reflected, {} spec FQCNs reflected",
        plugin_fqcns_to_reflect.len(),
        specs_needing_reflection.len(),
    );

    // -----------------------------------------------------------------
    // Phase C: apply — sequential, pure computation, no I/O.
    // self/static/parent and union types are already normalised by
    // normalize_reflected_method_signature above; class-constant
    // defaults come through via PHP reflection in Phase B.
    // -----------------------------------------------------------------
    for spec in specs.iter_mut() {
        let needs_sig = interceptor_methods_need_reflection_normalization(&spec.public_methods);
        let expected = if spec.plugins.is_empty() {
            FxHashSet::default()
        } else {
            compute_expected_method_names(&spec.plugins, class_map, di_config, &plugin_method_map)
        };

        if spec.plugins.is_empty() && !needs_sig {
            continue;
        }
        if !spec.plugins.is_empty() && expected.is_empty() && !needs_sig {
            continue;
        }

        let current: FxHashSet<String> = spec.public_methods.iter().map(|m| m.name.clone()).collect();
        let missing_expected = !spec.plugins.is_empty() && !expected.is_subset(&current);
        if !missing_expected && !needs_sig {
            continue;
        }

        let Some(reflected_methods) = spec_reflection_map.get(&spec.fqcn) else {
            continue;
        };

        let reflected_by_name: FxHashMap<&str, &MethodSignature> = reflected_methods
            .iter()
            .map(|m| (m.name.as_str(), m))
            .collect();

        if !missing_expected {
            // Keep resolver-selected method set/order; normalise signatures
            // from reflection (self→concrete, static→concrete, union order, defaults).
            spec.public_methods = spec
                .public_methods
                .iter()
                .map(|m| {
                    reflected_by_name
                        .get(m.name.as_str())
                        .copied()
                        .cloned()
                        .unwrap_or_else(|| m.clone())
                })
                .collect();
        } else {
            let filtered: Vec<MethodSignature> = reflected_methods
                .iter()
                .filter(|m| expected.contains(&m.name))
                .cloned()
                .collect();
            if !filtered.is_empty() {
                spec.public_methods = filtered;
            }
        }
    }
}

/// Returns true when `spec` needs PHP reflection — either its method signatures
/// contain types requiring normalisation (self/static/parent/union/constants) or
/// it has plugin registrations whose expected intercepted methods are not present.
fn spec_needs_reflection(
    spec: &di_resolver::InterceptorSpec,
    class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
    plugin_method_map: &FxHashMap<String, FxHashSet<String>>,
) -> bool {
    if interceptor_methods_need_reflection_normalization(&spec.public_methods) {
        return true;
    }
    if spec.plugins.is_empty() {
        return false;
    }
    let expected =
        compute_expected_method_names(&spec.plugins, class_map, di_config, plugin_method_map);
    if expected.is_empty() {
        return false;
    }
    let current: FxHashSet<&str> = spec
        .public_methods
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    !expected.iter().all(|e| current.contains(e.as_str()))
}

/// Compute the set of intercepted method names expected from a plugin list,
/// using class_map data and the precomputed plugin-method reflection map.
/// Pure computation — no I/O.
fn compute_expected_method_names(
    plugins: &[di_resolver::PluginRef],
    class_map: &FxHashMap<String, ClassInfo>,
    di_config: &DiConfig,
    plugin_method_map: &FxHashMap<String, FxHashSet<String>>,
) -> FxHashSet<String> {
    let mut names = FxHashSet::default();
    for plugin in plugins {
        let resolved = di_config
            .get_instance_type(&plugin.type_name)
            .trim_start_matches('\\')
            .to_string();
        let raw = plugin.type_name.trim_start_matches('\\').to_string();
        let mut candidates = vec![resolved, raw];
        candidates.sort();
        candidates.dedup();
        candidates.retain(|c| !c.is_empty());

        for candidate in &candidates {
            if let Some(plugin_info) = class_map.get(candidate) {
                for method in &plugin_info.public_methods {
                    if let Some(name) = plugin_method_to_intercepted(&method.name) {
                        names.insert(name);
                    }
                }
            }
            if let Some(reflected_names) = plugin_method_map.get(candidate) {
                names.extend(reflected_names.iter().cloned());
            }
        }
    }
    names
}

fn interceptor_methods_need_reflection_normalization(methods: &[MethodSignature]) -> bool {
    methods.iter().any(|method| {
        method
            .return_type
            .as_deref()
            .map(|rt| matches!(rt, "self" | "parent" | "static") || rt.contains('|'))
            .unwrap_or(false)
            || method.params.iter().any(|param| {
                param
                    .type_hint
                    .as_deref()
                    .map(|th| matches!(th, "self" | "parent" | "static") || th.contains('|'))
                    .unwrap_or(false)
                    || param
                        .default_value
                        .as_deref()
                        .map(|dv| {
                            let t = dv.trim();
                            t.contains("::")
                                || t.eq_ignore_ascii_case("null")
                                || t.starts_with("array (")
                        })
                        .unwrap_or(false)
            })
    })
}

fn normalize_reflected_method_signature(
    method: &mut MethodSignature,
    target_fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) {
    if let Some(rt) = method.return_type.as_mut() {
        *rt = normalize_reflected_type_hint(rt, target_fqcn, class_map);
    }
    for param in method.params.iter_mut() {
        if let Some(th) = param.type_hint.as_mut() {
            *th = normalize_reflected_type_hint(th, target_fqcn, class_map);
        }
    }
}

fn normalize_reflected_method_signature_for_proxy(
    method: &mut MethodSignature,
    target_fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) {
    if let Some(rt) = method.return_type.as_mut() {
        *rt = normalize_reflected_type_hint_for_proxy(rt, target_fqcn, class_map);
    }
    for param in method.params.iter_mut() {
        if let Some(th) = param.type_hint.as_mut() {
            *th = normalize_reflected_type_hint_for_proxy(th, target_fqcn, class_map);
        }
    }
}

fn normalize_reflected_type_hint(
    raw: &str,
    target_fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) -> String {
    const BUILTIN_PRECEDENCE: &[(&str, u8)] = &[
        ("bool", 1),
        ("int", 2),
        ("float", 3),
        ("string", 4),
        ("array", 5),
        ("callable", 6),
        ("iterable", 7),
        ("object", 8),
        ("static", 9),
        ("mixed", 10),
        ("void", 11),
        ("false", 12),
        ("true", 13),
        ("null", 14),
        ("never", 15),
    ];

    let normalized_target = target_fqcn.trim_start_matches('\\');
    let current_info = class_map.get(normalized_target);

    let (nullable, core) = if let Some(rest) = raw.strip_prefix('?') {
        ("?", rest)
    } else {
        ("", raw)
    };

    let normalize_single = |part: &str| -> String {
        match part {
            "self" | "static" => normalized_target.to_string(),
            "parent" => current_info
                .and_then(|info| info.extends.clone())
                .unwrap_or_else(|| "parent".to_string()),
            _ => part.trim_start_matches('\\').to_string(),
        }
    };

    if core.contains('|') {
        let mut parts: Vec<String> = core
            .split('|')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(normalize_single)
            .collect();
        let precedence_of = |value: &str| -> u8 {
            let lower = value.to_ascii_lowercase();
            BUILTIN_PRECEDENCE
                .iter()
                .find_map(|(name, rank)| (*name == lower).then_some(*rank))
                .unwrap_or(0)
        };
        parts.sort_by(|a, b| {
            precedence_of(a)
                .cmp(&precedence_of(b))
                .then_with(|| a.cmp(b))
        });
        return format!("{}{}", nullable, parts.join("|"));
    }

    format!("{}{}", nullable, normalize_single(core))
}

fn normalize_reflected_type_hint_for_proxy(
    raw: &str,
    target_fqcn: &str,
    class_map: &FxHashMap<String, ClassInfo>,
) -> String {
    const BUILTIN_PRECEDENCE: &[(&str, u8)] = &[
        ("bool", 1),
        ("int", 2),
        ("float", 3),
        ("string", 4),
        ("array", 5),
        ("callable", 6),
        ("iterable", 7),
        ("object", 8),
        ("static", 9),
        ("mixed", 10),
        ("void", 11),
        ("false", 12),
        ("true", 13),
        ("null", 14),
        ("never", 15),
    ];

    let normalized_target = target_fqcn.trim_start_matches('\\');
    let current_info = class_map.get(normalized_target);

    let (nullable, core) = if let Some(rest) = raw.strip_prefix('?') {
        ("?", rest)
    } else {
        ("", raw)
    };

    let normalize_single = |part: &str| -> String {
        match part {
            // Keep proxy parity with Magento: `self` resolves to concrete target
            // class name, while `static` remains `static`.
            "self" => normalized_target.to_string(),
            "parent" => current_info
                .and_then(|info| info.extends.clone())
                .unwrap_or_else(|| "parent".to_string()),
            _ => part.trim_start_matches('\\').to_string(),
        }
    };

    if core.contains('|') {
        let mut parts: Vec<String> = core
            .split('|')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(normalize_single)
            .collect();
        let precedence_of = |value: &str| -> u8 {
            let lower = value.to_ascii_lowercase();
            BUILTIN_PRECEDENCE
                .iter()
                .find_map(|(name, rank)| (*name == lower).then_some(*rank))
                .unwrap_or(0)
        };
        parts.sort_by(|a, b| {
            precedence_of(a)
                .cmp(&precedence_of(b))
                .then_with(|| a.cmp(b))
        });
        return format!("{}{}", nullable, parts.join("|"));
    }

    format!("{}{}", nullable, normalize_single(core))
}

/// Build a lowercase-key → canonical-case-FQCN index from the class map.
/// Real source classes (path.exists()) win over synthetic entries; ties
/// break lexicographically.  Build once and reuse across all areas.
fn build_case_index(class_map: &FxHashMap<String, ClassInfo>) -> FxHashMap<String, String> {
    let mut case_index: FxHashMap<String, (String, u8)> = FxHashMap::default();
    case_index.reserve(class_map.len());
    for (fqcn, info) in class_map {
        let key = fqcn.to_ascii_lowercase();
        // Prefer real source classes over synthetic/generated entries when
        // casing variants collide on the same lowercase FQCN.
        let rank = if info.path.exists() { 0 } else { 1 };
        match case_index.get_mut(&key) {
            None => {
                case_index.insert(key, (fqcn.clone(), rank));
            }
            Some((current, current_rank)) => {
                if rank < *current_rank || (rank == *current_rank && fqcn < current) {
                    *current = fqcn.clone();
                    *current_rank = rank;
                }
            }
        }
    }
    case_index.into_iter().map(|(k, (v, _))| (k, v)).collect()
}

/// Apply a pre-built case index to an args map (shared across all areas).
fn apply_case_index(
    args_map: &mut FxHashMap<String, Vec<di_resolver::ResolvedArg>>,
    case_index: &FxHashMap<String, String>,
) {
    for args in args_map.values_mut() {
        for arg in args.iter_mut() {
            canonicalize_resolved_arg_value_case(&mut arg.resolved, case_index);
        }
    }
}


fn canonicalize_resolved_arg_value_case(
    value: &mut di_resolver::ResolvedArgValue,
    case_index: &FxHashMap<String, String>,
) {
    use di_resolver::{ResolvedArgValue, ResolvedArrayValue, ResolvedScalar};

    match value {
        ResolvedArgValue::SharedInstance(fqcn) | ResolvedArgValue::NonSharedInstance(fqcn) => {
            canonicalize_fqcn_case(fqcn, case_index);
        }
        ResolvedArgValue::Array(items) => {
            for item in items.iter_mut() {
                canonicalize_resolved_arg_value_case(&mut item.resolved, case_index);
            }
        }
        ResolvedArgValue::PlainArray(items) => {
            for item in items.iter_mut() {
                if item.name == "instance" {
                    if let ResolvedArrayValue::Scalar(ResolvedScalar::String(v)) = &mut item.value {
                        canonicalize_fqcn_case(v, case_index);
                    }
                }
                if let ResolvedArrayValue::Array(children) = &mut item.value {
                    canonicalize_plain_array_case(children, case_index);
                }
            }
        }
        _ => {}
    }
}

fn canonicalize_plain_array_case(
    items: &mut Vec<di_resolver::ResolvedArrayItem>,
    case_index: &FxHashMap<String, String>,
) {
    use di_resolver::{ResolvedArrayValue, ResolvedScalar};

    for item in items.iter_mut() {
        if item.name == "instance" {
            if let ResolvedArrayValue::Scalar(ResolvedScalar::String(v)) = &mut item.value {
                canonicalize_fqcn_case(v, case_index);
            }
        }
        if let ResolvedArrayValue::Array(children) = &mut item.value {
            canonicalize_plain_array_case(children, case_index);
        }
    }
}

fn canonicalize_fqcn_case(value: &mut String, case_index: &FxHashMap<String, String>) {
    let lookup = value.trim_start_matches('\\').to_ascii_lowercase();
    if let Some(canonical) = case_index.get(&lookup) {
        *value = canonical.clone();
    }
}

fn plugin_method_to_intercepted(method: &str) -> Option<String> {
    if let Some(rest) = method.strip_prefix("before") {
        return lcfirst_nonempty(rest);
    }
    if let Some(rest) = method.strip_prefix("around") {
        return lcfirst_nonempty(rest);
    }
    if let Some(rest) = method.strip_prefix("after") {
        return lcfirst_nonempty(rest);
    }
    None
}

fn lcfirst_nonempty(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let first = chars.next()?;
    let mut out = first.to_lowercase().to_string();
    out.push_str(chars.as_str());
    Some(out)
}

fn reflect_interceptable_methods(
    fqcn: &str,
    _magento_root: &Path,
    _php_bin: &str,
) -> Option<Vec<MethodSignature>> {
    #[derive(Deserialize)]
    struct ReflectionParam {
        name: String,
        #[serde(default)]
        type_hint: Option<String>,
        #[serde(default)]
        has_default: bool,
        #[serde(default)]
        default_value: Option<String>,
        #[serde(default)]
        is_variadic: bool,
        #[serde(default)]
        is_by_ref: bool,
    }

    #[derive(Deserialize)]
    struct ReflectionMethod {
        name: String,
        #[serde(default)]
        params: Vec<ReflectionParam>,
        #[serde(default)]
        return_type: Option<String>,
        #[serde(default)]
        returns_reference: bool,
    }

    let pool = PHP_WORKER_POOL.get()?;
    let request = format!("methods:{}", fqcn.trim_start_matches('\\'));
    let json_raw = pool.request(&request)?;
    let json_trimmed = json_raw.trim();
    if json_trimmed == "null" || json_trimmed.is_empty() {
        return None;
    }
    let json_slice = if json_trimmed.starts_with('[') {
        json_trimmed
    } else {
        let start = json_trimmed.find('[')?;
        let end = json_trimmed.rfind(']')?;
        if end < start {
            return None;
        }
        &json_trimmed[start..=end]
    };
    let rows: Vec<ReflectionMethod> = serde_json::from_str(json_slice).ok()?;

    let mut out = Vec::with_capacity(rows.len());
    for method in rows {
        let params = method
            .params
            .into_iter()
            .map(|p| MethodParam {
                name: p.name,
                type_hint: p.type_hint.map(|t| t.trim_start_matches('\\').to_string()),
                has_default: p.has_default,
                default_value: p.default_value.map(|v| normalize_php_default_value(&v)),
                is_variadic: p.is_variadic,
                is_by_ref: p.is_by_ref,
            })
            .collect();
        out.push(MethodSignature {
            name: method.name,
            params,
            return_type: method
                .return_type
                .map(|t| t.trim_start_matches('\\').to_string()),
            is_static: false,
            returns_reference: method.returns_reference,
        });
    }
    Some(out)
}

fn reflect_class_kind(fqcn: &str, _magento_root: &Path, _php_bin: &str) -> Option<ClassKind> {
    let pool = PHP_WORKER_POOL.get()?;
    let request = format!("kind:{}", fqcn.trim_start_matches('\\'));
    let json_raw = pool.request(&request)?;
    let json_trimmed = json_raw.trim();
    if json_trimmed == "null" || json_trimmed.is_empty() {
        return None;
    }
    let kind: String = serde_json::from_str(json_trimmed).ok()?;
    match kind.as_str() {
        "interface" => Some(ClassKind::Interface),
        "trait" => Some(ClassKind::Trait),
        "class" => Some(ClassKind::Class),
        _ => None,
    }
}

fn normalize_php_default_value(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return "null".to_string();
    }
    if let Some(inner) = trimmed
        .strip_prefix("array (")
        .and_then(|v| v.strip_suffix(')'))
    {
        if inner.trim().is_empty() {
            return "[]".to_string();
        }
    }
    trimmed.to_string()
}

fn constructor_params_need_reflection(params: &[ConstructorParam]) -> bool {
    params.iter().any(|param| {
        param
            .type_hint
            .as_deref()
            .map(|th| matches!(th, "self" | "parent" | "static"))
            .unwrap_or(false)
            || param
                .default_value
                .as_deref()
                .map(|dv| {
                    let t = dv.trim();
                    t.contains("::") || t.eq_ignore_ascii_case("null") || t.starts_with("array (")
                })
                .unwrap_or(false)
    })
}

fn reflect_constructor_params(
    fqcn: &str,
    _magento_root: &Path,
    _php_bin: &str,
) -> Option<Vec<ConstructorParam>> {
    #[derive(Deserialize)]
    struct ReflectionParam {
        name: String,
        #[serde(default)]
        type_hint: Option<String>,
        #[serde(default)]
        has_default: bool,
        #[serde(default)]
        default_value: Option<String>,
        #[serde(default)]
        is_variadic: bool,
    }

    let pool = PHP_WORKER_POOL.get()?;
    let request = format!("ctor:{}", fqcn.trim_start_matches('\\'));
    let json_raw = pool.request(&request)?;
    let json_trimmed = json_raw.trim();
    if json_trimmed == "null" || json_trimmed.is_empty() {
        return None;
    }
    let json_slice = if json_trimmed.starts_with('[') {
        json_trimmed
    } else {
        let start = json_trimmed.find('[')?;
        let end = json_trimmed.rfind(']')?;
        if end < start {
            return None;
        }
        &json_trimmed[start..=end]
    };

    let rows: Option<Vec<ReflectionParam>> = serde_json::from_str(json_slice).ok()?;
    let rows = rows?;
    Some(
        rows.into_iter()
            .map(|p| ConstructorParam {
                name: p.name,
                type_hint: p.type_hint.map(|t| t.trim_start_matches('\\').to_string()),
                is_optional: p.has_default,
                default_value: p.default_value.map(|v| normalize_php_default_value(&v)),
                is_primitive: false,
                is_variadic: p.is_variadic,
                is_promoted: false,
            })
            .collect(),
    )
}

fn synthetic_search_results_class_info(spec: &SearchResultsSpec) -> ClassInfo {
    let normalized = spec.result_fqcn.trim_start_matches('\\');
    let (namespace, name) = if let Some((ns, class_name)) = normalized.rsplit_once('\\') {
        (ns.to_string(), class_name.to_string())
    } else {
        (String::new(), normalized.to_string())
    };

    ClassInfo {
        path: PathBuf::from("__generated__/search_results.php"),
        namespace,
        name,
        fqcn: normalized.to_string(),
        kind: ClassKind::Class,
        extends: Some("Magento\\Framework\\Api\\SearchResults".to_string()),
        implements: vec![],
        constructor: None,
        is_abstract: false,
        is_final: false,
        public_methods: vec![MethodSignature {
            name: "getItems".to_string(),
            params: vec![],
            return_type: None,
            is_static: false,
            returns_reference: false,
        }],
    }
}

fn synthetic_proxy_deferred_class_info(spec: &ProxyDeferredSpec) -> ClassInfo {
    let normalized = spec.proxy_fqcn.trim_start_matches('\\');
    let (namespace, name) = if let Some((ns, class_name)) = normalized.rsplit_once('\\') {
        (ns.to_string(), class_name.to_string())
    } else {
        (String::new(), normalized.to_string())
    };

    ClassInfo {
        path: PathBuf::from("__generated__/proxy_deferred.php"),
        namespace,
        name,
        fqcn: normalized.to_string(),
        kind: ClassKind::Class,
        extends: Some(spec.target_fqcn.clone()),
        implements: vec![
            "Magento\\Framework\\ObjectManager\\NoninterceptableInterface".to_string(),
        ],
        constructor: None,
        is_abstract: false,
        is_final: false,
        public_methods: vec![],
    }
}

fn synthetic_proxy_target_info(target_fqcn: &str, kind: ClassKind) -> ClassInfo {
    let normalized = target_fqcn.trim_start_matches('\\');
    let (namespace, name) = if let Some((ns, class_name)) = normalized.rsplit_once('\\') {
        (ns.to_string(), class_name.to_string())
    } else {
        (String::new(), normalized.to_string())
    };

    ClassInfo {
        path: PathBuf::from("__generated__/proxy_target.php"),
        namespace,
        name,
        fqcn: normalized.to_string(),
        kind,
        extends: None,
        implements: vec![],
        constructor: None,
        is_abstract: false,
        is_final: false,
        public_methods: vec![],
    }
}

fn plugin_list_cache_id(scope: &str) -> String {
    if scope == "global" {
        "primary|global|plugin-list".to_string()
    } else {
        format!("primary|global|{}|plugin-list", scope)
    }
}

fn find_extension_attributes_files(magento_root: &Path, module_paths: &[PathBuf]) -> Vec<PathBuf> {
    // Magento composes XML by module load order (app/etc/config.php), not by
    // filesystem path order. Preserve that so generated Extension* method order
    // matches PHP compile output.
    let module_order = load_module_order_from_config_php(magento_root);
    let mut ordered_files: Vec<(usize, usize, PathBuf)> = Vec::new();
    for (discovery_idx, module_path) in module_paths.iter().enumerate() {
        let file = module_path.join("etc/extension_attributes.xml");
        if file.exists() {
            let order = read_module_name_from_module_xml(module_path)
                .and_then(|name| module_order.get(&name).copied())
                .unwrap_or(usize::MAX / 2 + discovery_idx);
            ordered_files.push((order, discovery_idx, file));
        }
    }
    ordered_files.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    let mut files = Vec::new();
    let mut seen = FxHashSet::default();
    for (_, _, file) in ordered_files {
        if seen.insert(file.clone()) {
            files.push(file);
        }
    }
    let app_etc = magento_root.join("app/etc/extension_attributes.xml");
    if app_etc.exists() && seen.insert(app_etc.clone()) {
        files.push(app_etc);
    }
    files
}

/// TKT-054: Bootstrap PHP extension/built-in constants into the const_map.
///
/// PHP extension constants like `MCRYPT_BLOWFISH` or `MCRYPT_MODE_ECB` are
/// never defined in any Magento PHP source file, so they never enter the
/// source-scan const_map and get emitted verbatim (causing metadata mismatches).
///
/// This function runs `php -r` once at startup with `get_defined_constants(true)`
/// and collects all scalar constants as a baseline. Source-scan constants added
/// later override these builtins on name collision.
fn bootstrap_php_constants(php_bin: &str, magento_root: &Path) -> FxHashMap<String, String> {
    // Include vendor/autoload.php so that Composer-autoloaded constants (e.g. MCRYPT_BLOWFISH
    // defined by phpseclib/mcrypt_compat) are present before we call get_defined_constants().
    let autoloader = magento_root.join("vendor/autoload.php");
    let autoload_snippet = if autoloader.exists() {
        format!(
            "@require '{}';",
            autoloader.to_string_lossy().replace('\'', "\\'")
        )
    } else {
        String::new()
    };
    let collect = concat!(
        "$c=get_defined_constants(true);",
        "$o=[];",
        "foreach($c as $items){",
        "  foreach($items as $k=>$v){",
        "    if(is_scalar($v)) $o[$k]=(string)$v;",
        "  }",
        "}",
        "echo json_encode($o);"
    );
    let script = format!("{autoload_snippet}{collect}");
    let output = std::process::Command::new(php_bin)
        .args(["-r", &script])
        .output();
    let Ok(out) = output else {
        return FxHashMap::default();
    };
    if !out.status.success() {
        return FxHashMap::default();
    }
    serde_json::from_slice(&out.stdout).unwrap_or_default()
}

fn load_module_order_from_config_php(magento_root: &Path) -> FxHashMap<String, usize> {
    let mut out = FxHashMap::default();
    let config_php = magento_root.join("app/etc/config.php");
    let Ok(content) = std::fs::read_to_string(config_php) else {
        return out;
    };

    let mut in_modules = false;
    let mut idx = 0usize;
    for raw in content.lines() {
        let line = raw.trim();
        if !in_modules {
            if line.starts_with("'modules'") || line.starts_with("\"modules\"") {
                in_modules = true;
            }
            continue;
        }
        if line.starts_with("),") || line.starts_with("]") || line == ")" || line == "]" {
            break;
        }
        if !line.contains("=>") {
            continue;
        }
        let Some((quote_pos, quote_ch)) = line
            .char_indices()
            .find(|(_, ch)| *ch == '\'' || *ch == '"')
        else {
            continue;
        };
        let rest = &line[quote_pos + quote_ch.len_utf8()..];
        let Some(end_rel) = rest.find(quote_ch) else {
            continue;
        };
        let module = &rest[..end_rel];
        if module.is_empty() || out.contains_key(module) {
            continue;
        }
        // Config line format: 'ModuleName' => 1, or 'ModuleName' => 0,
        let after_key = &rest[end_rel + quote_ch.len_utf8()..];
        let enabled = after_key
            .split("=>")
            .nth(1)
            .map(|v| v.trim().trim_end_matches([',', ' ', '\n']).trim() != "0")
            .unwrap_or(true);
        // TKT-048: Always increment idx for every module (enabled or disabled) so
        // that positional indices reflect true config.php insertion order.
        // Only enabled modules are inserted into the output map, but their index
        // must account for preceding disabled entries so that when two modules have
        // equal DI priority the module appearing later in config.php wins.
        let current_idx = idx;
        idx += 1;
        if !enabled {
            continue;
        }
        out.insert(module.to_string(), current_idx);
    }

    out
}

/// Return the module root directory for a di.xml path by walking up to find the `etc` parent.
fn module_root_from_di_xml(di_xml: &Path) -> Option<&Path> {
    // A di.xml path is either:
    //   <root>/etc/di.xml
    //   <root>/etc/<area>/di.xml
    // Walk up until we find a component named "etc" and return its parent.
    let mut p = di_xml.parent()?;
    loop {
        if p.file_name()?.to_str()? == "etc" {
            return p.parent();
        }
        p = p.parent()?;
    }
}

/// Vendor packages that must always be excluded from DI config regardless of
/// whether they have a `registration.php`. These are test-only or dev-only
/// frameworks that ship a di.xml but must never contribute to production DI.
///
/// TKT-051: explicit denylist prevents accidental inclusion of testing frameworks
/// that may have a `registration.php` but are not production Magento modules.
const EXCLUDED_PACKAGES: &[&str] = &["magento2-functional-testing-framework", "mftf"];

/// Filter a list of di.xml paths, dropping files from disabled modules.
///
/// A di.xml is dropped when its module can be identified via `etc/module.xml`
/// AND that module name is absent from `enabled_modules` (which only contains
/// enabled modules from `app/etc/config.php`).
///
/// For vendor packages that have neither `etc/module.xml` nor `registration.php`,
/// the file is dropped — these are utility/testing packages (e.g. functional-testing-framework)
/// that are not Magento modules and should not contribute to production DI config.
///
/// Files at `app/etc/di.xml` (no vendor module root) are always kept.
fn filter_enabled_di_xml(
    files: Vec<PathBuf>,
    enabled_modules: &FxHashMap<String, usize>,
    magento_root: &Path,
) -> Vec<PathBuf> {
    files
        .into_iter()
        .filter(|path| {
            // TKT-051: Always exclude test-only packages by path denylist.
            let path_str = path.to_string_lossy();
            if EXCLUDED_PACKAGES.iter().any(|pkg| path_str.contains(pkg)) {
                return false;
            }

            let Some(module_root) = module_root_from_di_xml(path) else {
                return true; // can't determine module root → keep (e.g. app/etc/di.xml)
            };
            // Check if this is a vendor package with module identity.
            let is_vendor = is_vendor_package_root(module_root, magento_root);
            if let Some(name) = read_module_name_from_module_xml(module_root) {
                // Has module.xml — enabled check applies
                return enabled_modules.contains_key(&name);
            }
            if is_vendor {
                // Vendor package with no module.xml — check for registration.php.
                // Packages without registration.php are not Magento modules (e.g. testing frameworks).
                return module_root.join("registration.php").exists();
            }
            true // non-vendor path with no module.xml → keep
        })
        .collect()
}

fn is_vendor_package_root(module_root: &Path, magento_root: &Path) -> bool {
    // Expected shape: <magento-root>/vendor/<vendor>/<package>
    if let Ok(rel) = module_root.strip_prefix(magento_root) {
        let mut comps = rel.components();
        return comps.next().and_then(|c| c.as_os_str().to_str()) == Some("vendor")
            && comps.next().is_some()
            && comps.next().is_some();
    }

    // Fallback for unexpected path roots.
    let parts: Vec<&str> = module_root
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    parts.windows(3).any(|w| w[0] == "vendor")
}

fn read_module_name_from_module_xml(module_root: &Path) -> Option<String> {
    let module_xml = module_root.join("etc/module.xml");
    let content = std::fs::read_to_string(module_xml).ok()?;
    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e))
                if local_name(e.name().as_ref()) == "module" =>
            {
                if let Some(name) = event_attr(e, b"name") {
                    let normalized = name.trim().to_string();
                    if !normalized.is_empty() {
                        return Some(normalized);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

fn parse_extension_attributes_files(
    files: &[PathBuf],
) -> FxHashMap<String, Vec<ExtensionAttributeSpec>> {
    let mut out: FxHashMap<String, Vec<ExtensionAttributeSpec>> = FxHashMap::default();
    for file in files {
        let Ok(content) = std::fs::read_to_string(file) else {
            continue;
        };
        let mut reader = Reader::from_str(&content);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut current_for: Option<String> = None;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    if local_name(e.name().as_ref()) == "extension_attributes" {
                        current_for = event_attr(e, b"for")
                            .map(|v| v.trim().trim_start_matches('\\').to_string());
                    } else if local_name(e.name().as_ref()) == "attribute" {
                        maybe_collect_extension_attribute(e, current_for.as_deref(), &mut out);
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    if local_name(e.name().as_ref()) == "attribute" {
                        maybe_collect_extension_attribute(e, current_for.as_deref(), &mut out);
                    }
                }
                Ok(Event::End(ref e)) => {
                    if local_name(e.name().as_ref()) == "extension_attributes" {
                        current_for = None;
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    for attrs in out.values_mut() {
        *attrs = dedupe_attributes_keep_last(attrs.clone());
    }

    out
}

fn maybe_collect_extension_attribute(
    e: &quick_xml::events::BytesStart<'_>,
    current_for: Option<&str>,
    out: &mut FxHashMap<String, Vec<ExtensionAttributeSpec>>,
) {
    let Some(source_interface) = current_for else {
        return;
    };
    let Some(code) = event_attr(e, b"code") else {
        return;
    };
    let Some(php_type) = event_attr(e, b"type") else {
        return;
    };
    if code.trim().is_empty() || php_type.trim().is_empty() {
        return;
    }
    out.entry(source_interface.to_string())
        .or_default()
        .push(ExtensionAttributeSpec {
            code: code.trim().to_string(),
            php_type: php_type.trim().to_string(),
        });
}

fn dedupe_attributes_keep_last(attrs: Vec<ExtensionAttributeSpec>) -> Vec<ExtensionAttributeSpec> {
    let mut seen = FxHashSet::default();
    let mut out = Vec::new();
    for attr in attrs.into_iter().rev() {
        if seen.insert(attr.code.clone()) {
            out.push(attr);
        }
    }
    out.reverse();
    out
}

fn event_attr(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    for a in e.attributes().flatten() {
        if a.key.as_ref() == key {
            return std::str::from_utf8(a.value.as_ref())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

fn local_name(name: &[u8]) -> &str {
    let bytes = match name.iter().position(|&b| b == b':') {
        Some(pos) => &name[pos + 1..],
        None => name,
    };
    std::str::from_utf8(bytes).unwrap_or("")
}

fn collect_extension_specs(
    class_map: &FxHashMap<String, ClassInfo>,
    xml_attrs: &FxHashMap<String, Vec<ExtensionAttributeSpec>>,
) -> Vec<ExtensionSpec> {
    let mut specs_by_interface: FxHashMap<String, ExtensionSpec> = FxHashMap::default();

    // Path 1: extension_attributes.xml declarations.
    let mut xml_keys: Vec<&String> = xml_attrs.keys().collect();
    xml_keys.sort();
    for source_interface in xml_keys {
        if !class_map.contains_key(source_interface.as_str()) {
            continue;
        }
        let Some((ext_interface, ext_class)) =
            derive_extension_names_from_source_interface(source_interface)
        else {
            continue;
        };
        specs_by_interface.insert(
            ext_interface.clone(),
            ExtensionSpec {
                source_interface_fqcn: source_interface.clone(),
                extension_interface_fqcn: ext_interface,
                extension_class_fqcn: ext_class,
                attributes: xml_attrs.get(source_interface).cloned().unwrap_or_default(),
            },
        );
    }

    // Path 2: PHP interface method pattern (getExtensionAttributes return type).
    for info in class_map.values() {
        if info.kind != ClassKind::Interface {
            continue;
        }
        let has_get_extension_attrs = info
            .public_methods
            .iter()
            .any(|m| m.name == "getExtensionAttributes");
        if !has_get_extension_attrs {
            continue;
        }

        let inferred_from_signature = info
            .public_methods
            .iter()
            .find(|m| m.name == "getExtensionAttributes")
            .and_then(|m| m.return_type.as_deref())
            .and_then(first_non_null_type_hint_arm)
            .filter(|t| t.ends_with("ExtensionInterface"));
        let inferred_from_docblock =
            parse_get_extension_attributes_return_from_docblock(&info.path)
                .and_then(|r| first_non_null_type_hint_arm(&r))
                .filter(|t| t.ends_with("ExtensionInterface"));
        let Some(ext_interface) = inferred_from_signature.or(inferred_from_docblock) else {
            continue;
        };
        let ext_class = if ext_interface.ends_with("Interface") {
            ext_interface[..ext_interface.len() - "Interface".len()].to_string()
        } else {
            continue;
        };
        let source_interface = info.fqcn.clone();

        specs_by_interface
            .entry(ext_interface.clone())
            .and_modify(|spec| {
                // Keep the first resolved owner by default (XML path is already
                // deterministic and should remain authoritative). Only replace
                // when we can enrich an empty-attributes spec from XML.
                if spec.attributes.is_empty() {
                    if let Some(attrs) = xml_attrs.get(&source_interface) {
                        spec.source_interface_fqcn = source_interface.clone();
                        spec.extension_class_fqcn = ext_class.clone();
                        spec.attributes = attrs.clone();
                    }
                }
            })
            .or_insert_with(|| ExtensionSpec {
                source_interface_fqcn: source_interface.clone(),
                extension_interface_fqcn: ext_interface,
                extension_class_fqcn: ext_class,
                attributes: xml_attrs
                    .get(&source_interface)
                    .cloned()
                    .unwrap_or_default(),
            });
    }

    let mut specs: Vec<ExtensionSpec> = specs_by_interface.into_values().collect();
    specs.sort_by(|a, b| {
        a.extension_interface_fqcn
            .cmp(&b.extension_interface_fqcn)
            .then(a.extension_class_fqcn.cmp(&b.extension_class_fqcn))
    });
    specs
}

fn derive_extension_names_from_source_interface(
    source_interface: &str,
) -> Option<(String, String)> {
    let suffix = "Interface";
    if !source_interface.ends_with(suffix) {
        return None;
    }
    let base = &source_interface[..source_interface.len() - suffix.len()];
    Some((
        format!("{base}ExtensionInterface"),
        format!("{base}Extension"),
    ))
}

fn first_non_null_type_hint_arm(type_hint: &str) -> Option<String> {
    type_hint
        .split('|')
        .map(str::trim)
        .map(|arm| arm.trim_start_matches('?').trim_start_matches('\\'))
        .find(|arm| !arm.is_empty() && !matches!(*arm, "null" | "false" | "true"))
        .map(ToOwned::to_owned)
}

fn parse_get_extension_attributes_return_from_docblock(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let func_pos = content.find("function getExtensionAttributes")?;
    let doc_start = content[..func_pos].rfind("/**")?;
    let doc_end = content[doc_start..func_pos].rfind("*/")? + doc_start;
    if doc_end <= doc_start {
        return None;
    }
    let doc = &content[doc_start..doc_end];
    for raw_line in doc.lines() {
        let line = raw_line.trim().trim_start_matches('*').trim();
        if let Some(rest) = line.strip_prefix("@return") {
            let token = rest.split_whitespace().next()?.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

#[derive(Default)]
struct ComposerAutoloadIndex {
    psr4_prefixes: Vec<(String, Vec<PathBuf>)>,
    psr0_prefixes: Vec<(String, Vec<PathBuf>)>,
}

impl ComposerAutoloadIndex {
    fn from_magento_root(magento_root: &Path) -> Option<Self> {
        let installed_json_path = magento_root.join("vendor/composer/installed.json");
        let installed_json = std::fs::read_to_string(&installed_json_path).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&installed_json).ok()?;

        let packages = parsed
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .or_else(|| parsed.as_array())?;

        let base_dir = installed_json_path.parent()?;
        let mut index = ComposerAutoloadIndex::default();

        for pkg in packages {
            let install_path = pkg
                .get("install-path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if install_path.is_empty() {
                continue;
            }
            let package_root = base_dir.join(install_path);
            let autoload = match pkg.get("autoload").and_then(serde_json::Value::as_object) {
                Some(a) => a,
                None => continue,
            };

            if let Some(psr4) = autoload.get("psr-4") {
                push_autoload_map(psr4, &package_root, &mut index.psr4_prefixes);
            }
            if let Some(psr0) = autoload.get("psr-0") {
                push_autoload_map(psr0, &package_root, &mut index.psr0_prefixes);
            }
        }

        // Longest prefix first for deterministic matching.
        index
            .psr4_prefixes
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        index
            .psr0_prefixes
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));

        Some(index)
    }

    fn is_loadable(&self, fqcn: &str) -> bool {
        self.resolve_class_path(fqcn).is_some()
    }

    fn resolve_class_path(&self, fqcn: &str) -> Option<PathBuf> {
        let fqcn = fqcn.trim().trim_start_matches('\\');
        if fqcn.is_empty() {
            return None;
        }

        // PSR-4
        for (prefix, dirs) in &self.psr4_prefixes {
            let Some(relative) = fqcn.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            let rel_path = relative.replace('\\', "/");
            for dir in dirs {
                let candidate = dir.join(format!("{rel_path}.php"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }

        // PSR-0
        for (prefix, dirs) in &self.psr0_prefixes {
            let Some(relative) = fqcn.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            let rel_path = relative.replace(['\\', '_'], "/");
            for dir in dirs {
                let candidate = dir.join(format!("{rel_path}.php"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }

        None
    }
}

fn push_autoload_map(
    autoload_value: &serde_json::Value,
    package_root: &Path,
    out: &mut Vec<(String, Vec<PathBuf>)>,
) {
    let Some(map) = autoload_value.as_object() else {
        return;
    };

    for (prefix, locations) in map {
        let prefix = prefix.trim().trim_start_matches('\\').to_string();

        let dirs: Vec<PathBuf> = match locations {
            serde_json::Value::String(s) => vec![package_root.join(s)],
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(|s| package_root.join(s))
                .collect(),
            _ => Vec::new(),
        };

        if !dirs.is_empty() {
            out.push((prefix, dirs));
        }
    }
}

fn collect_proxy_targets_from_di_configs(di_configs: &[DiConfig]) -> FxHashSet<String> {
    let mut targets = FxHashSet::default();
    for cfg in di_configs {
        for proxy_fqcn in cfg.preferences.values() {
            maybe_push_proxy_target(proxy_fqcn, &mut targets);
        }
        for vt in cfg.virtual_types.values() {
            maybe_push_proxy_target(&vt.type_name, &mut targets);
        }
        for tc in cfg.type_configs.values() {
            collect_proxy_targets_from_args(&tc.arguments, &mut targets);
        }
    }
    targets
}

fn collect_proxy_targets_from_args(args: &[Argument], out: &mut FxHashSet<String>) {
    for arg in args {
        match arg {
            Argument::Object { value, .. } => maybe_push_proxy_target(value, out),
            Argument::Array { items, .. } => collect_proxy_targets_from_args(items, out),
            _ => {}
        }
    }
}

fn maybe_push_proxy_target(candidate: &str, out: &mut FxHashSet<String>) {
    let candidate = candidate.trim().trim_start_matches('\\');
    if let Some(target) = candidate.strip_suffix("\\Proxy") {
        if !target.is_empty() {
            out.insert(target.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_setup_di_compile_runtime_overrides, build_argument_type_names,
        build_interception_registry, build_interception_type_names,
        apply_case_index, build_case_index, infer_comparable_fix_categories,
        render_comparable_metadata_report_text, ComparableMetadataReport, ComparableReportSummary,
        ComparableTypeMismatchSample, ComparableValueMismatchSample,
    };
    use di_resolver::{
        ResolvedArg, ResolvedArgValue, ResolvedArrayItem, ResolvedArrayValue, ResolvedScalar,
    };
    use di_xml_reader::{Argument, DiConfig, VirtualType};
    use php_extractor::types::{ClassInfo, ClassKind};
    use rustc_hash::FxHashMap;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn class_info(fqcn: &str, extends: Option<&str>, is_abstract: bool) -> ClassInfo {
        class_info_impl(fqcn, extends, is_abstract, vec![])
    }

    fn class_info_with_implements(
        fqcn: &str,
        extends: Option<&str>,
        implements: Vec<&str>,
    ) -> ClassInfo {
        class_info_impl(fqcn, extends, false, implements.iter().map(|s| s.to_string()).collect())
    }

    fn class_info_impl(
        fqcn: &str,
        extends: Option<&str>,
        is_abstract: bool,
        implements: Vec<String>,
    ) -> ClassInfo {
        let (namespace, name) = fqcn
            .rsplit_once('\\')
            .map(|(ns, class)| (ns.to_string(), class.to_string()))
            .unwrap_or_else(|| (String::new(), fqcn.to_string()));
        ClassInfo {
            path: PathBuf::from("__test__.php"),
            namespace,
            name,
            fqcn: fqcn.to_string(),
            kind: if is_abstract {
                ClassKind::AbstractClass
            } else {
                ClassKind::Class
            },
            extends: extends.map(str::to_string),
            implements,
            constructor: None,
            is_abstract,
            is_final: false,
            public_methods: vec![],
        }
    }

    #[test]
    fn argument_type_names_include_vt_when_direct_type_is_intercepted() {
        let mut base_class_map = FxHashMap::default();
        base_class_map.insert(
            "Vendor\\Payment\\Adapter".to_string(),
            class_info("Vendor\\Payment\\Adapter", None, false),
        );
        let generated_class_map: FxHashMap<String, ClassInfo> = FxHashMap::default();

        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "VendorPaymentFacade".to_string(),
            VirtualType {
                name: "VendorPaymentFacade".to_string(),
                type_name: "Vendor\\Payment\\Adapter".to_string(),
            },
        );
        di_config
            .type_configs
            .entry("VendorPaymentFacade".to_string())
            .or_default();

        let interceptors = vec![di_resolver::InterceptorSpec {
            fqcn: "Vendor\\Payment\\Adapter".to_string(),
            plugins: vec![],
            public_methods: vec![],
        }];
        let factories: Vec<di_resolver::FactorySpec> = Vec::new();
        let proxies: Vec<di_resolver::ProxySpec> = Vec::new();
        let search_results: Vec<super::SearchResultsSpec> = Vec::new();
        let proxy_deferred: Vec<super::ProxyDeferredSpec> = Vec::new();
        let extension_specs: Vec<code_generator::ExtensionSpec> = Vec::new();

        let names = build_argument_type_names(
            &base_class_map,
            &generated_class_map,
            &di_config,
            &interceptors,
            &factories,
            &proxies,
            &search_results,
            &proxy_deferred,
            &extension_specs,
        );

        assert!(names.contains(&"VendorPaymentFacade".to_string()));
    }

    #[test]
    fn interception_type_names_filter_unknown_preference_types() {
        let mut base_class_map = FxHashMap::default();
        base_class_map.insert(
            "Known\\Concrete".to_string(),
            class_info("Known\\Concrete", None, false),
        );
        let generated_class_map: FxHashMap<String, ClassInfo> = FxHashMap::default();

        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "Known\\Virtual".to_string(),
            VirtualType {
                name: "Known\\Virtual".to_string(),
                type_name: "Known\\Concrete".to_string(),
            },
        );
        di_config.preferences.insert(
            "Unknown\\Interface".to_string(),
            "Unknown\\Implementation".to_string(),
        );
        di_config
            .preferences
            .insert("Known\\Virtual".to_string(), "Unknown\\Alias".to_string());
        di_config.preferences.insert(
            "Unknown\\External".to_string(),
            "Known\\Concrete".to_string(),
        );

        let interceptors: Vec<di_resolver::InterceptorSpec> = Vec::new();
        let factories: Vec<di_resolver::FactorySpec> = Vec::new();
        let proxies: Vec<di_resolver::ProxySpec> = Vec::new();
        let search_results: Vec<super::SearchResultsSpec> = Vec::new();
        let proxy_deferred: Vec<super::ProxyDeferredSpec> = Vec::new();
        let extension_specs: Vec<code_generator::ExtensionSpec> = Vec::new();

        let names = build_interception_type_names(
            &base_class_map,
            &generated_class_map,
            &di_config,
            &interceptors,
            &factories,
            &proxies,
            &search_results,
            &proxy_deferred,
            &extension_specs,
        );

        assert!(names.contains(&"Known\\Concrete".to_string()));
        assert!(names.contains(&"Known\\Virtual".to_string()));
        assert!(!names.contains(&"Unknown\\Interface".to_string()));
        assert!(!names.contains(&"Unknown\\Implementation".to_string()));
        assert!(!names.contains(&"Unknown\\Alias".to_string()));
        assert!(!names.contains(&"Unknown\\External".to_string()));
    }

    #[test]
    fn interception_registry_marks_virtual_types_and_ancestors_true() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Vendor\\AbstractParent".to_string(),
            class_info("Vendor\\AbstractParent", None, true),
        );
        class_map.insert(
            "Vendor\\ConcreteChild".to_string(),
            class_info(
                "Vendor\\ConcreteChild",
                Some("Vendor\\AbstractParent"),
                false,
            ),
        );

        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "Vendor\\Facade".to_string(),
            VirtualType {
                name: "Vendor\\Facade".to_string(),
                type_name: "Vendor\\ConcreteChild".to_string(),
            },
        );
        di_config.virtual_types.insert(
            "Vendor\\FacadeAlias".to_string(),
            VirtualType {
                name: "Vendor\\FacadeAlias".to_string(),
                type_name: "Vendor\\Facade".to_string(),
            },
        );

        let interceptors = vec![di_resolver::InterceptorSpec {
            fqcn: "Vendor\\ConcreteChild".to_string(),
            plugins: vec![],
            public_methods: vec![],
        }];
        let type_names = vec![
            "Vendor\\AbstractParent".to_string(),
            "Vendor\\ConcreteChild".to_string(),
            "Vendor\\Facade".to_string(),
            "Vendor\\FacadeAlias".to_string(),
            "Vendor\\Unrelated".to_string(),
        ];

        let proxies: Vec<di_resolver::ProxySpec> = Vec::new();
        let proxy_deferred: Vec<super::ProxyDeferredSpec> = Vec::new();
        let map = build_interception_registry(
            &type_names,
            &interceptors,
            &proxies,
            &proxy_deferred,
            &di_config,
            &class_map,
        );
        assert_eq!(map.get("Vendor\\AbstractParent"), Some(&true));
        assert_eq!(map.get("Vendor\\ConcreteChild"), Some(&true));
        assert_eq!(map.get("Vendor\\Facade"), Some(&true));
        assert_eq!(map.get("Vendor\\FacadeAlias"), Some(&true));
        assert_eq!(map.get("Vendor\\Unrelated"), Some(&false));
    }

    #[test]
    fn infer_categories_identifies_common_parity_buckets() {
        let report = ComparableMetadataReport {
            file: "primary|global|plugin-list.php".to_string(),
            summary: ComparableReportSummary::default(),
            sections: BTreeMap::new(),
            type_mismatches_by_pair: BTreeMap::from([
                ("string|number".to_string(), 12usize),
                ("NULL|object".to_string(), 2usize),
                ("object|array".to_string(), 1usize),
            ]),
            high_risk_mismatches_sample: Vec::new(),
            missing_paths_sample: vec![
                "arguments.AssetPreProcessor.candidates".to_string(),
                "preferences.SomeType".to_string(),
            ],
            extra_paths_sample: vec!["instanceTypes.SomeType".to_string()],
            value_mismatches_sample: Vec::new(),
            severity_score: 0,
        };

        let categories = infer_comparable_fix_categories(&report);
        assert!(categories
            .iter()
            .any(|c| c.contains("argument merge/resolution parity")));
        assert!(categories
            .iter()
            .any(|c| c.contains("instanceTypes mapping parity")));
        assert!(categories.iter().any(|c| c.contains("preferences parity")));
        assert!(categories
            .iter()
            .any(|c| c.contains("scalar normalization parity")));
        assert!(categories
            .iter()
            .any(|c| c.contains("array/object shape parity")));
        assert!(categories
            .iter()
            .any(|c| c.contains("null-container mismatch parity")));
        assert!(categories
            .iter()
            .any(|c| c.contains("plugin list key/surface parity")));
    }

    #[test]
    fn text_report_renders_expected_sections() {
        let report = ComparableMetadataReport {
            file: "adminhtml.php".to_string(),
            summary: ComparableReportSummary {
                missing_paths: 5,
                extra_paths: 3,
                type_mismatches: 2,
                value_mismatches: 1,
                high_risk_mismatches: 1,
            },
            sections: BTreeMap::from([(
                "arguments".to_string(),
                ComparableReportSummary {
                    missing_paths: 5,
                    extra_paths: 1,
                    type_mismatches: 2,
                    value_mismatches: 0,
                    high_risk_mismatches: 1,
                },
            )]),
            type_mismatches_by_pair: BTreeMap::from([("string|number".to_string(), 2usize)]),
            high_risk_mismatches_sample: vec![ComparableTypeMismatchSample {
                path: "arguments.foo".to_string(),
                truth_type: "NULL".to_string(),
                output_type: "object".to_string(),
                pair: "NULL|object".to_string(),
            }],
            missing_paths_sample: vec!["arguments.foo".to_string()],
            extra_paths_sample: vec!["instanceTypes.Bar".to_string()],
            value_mismatches_sample: vec![ComparableValueMismatchSample {
                path: "preferences.Foo".to_string(),
                truth: "\"A\"".to_string(),
                output: "\"B\"".to_string(),
            }],
            severity_score: 123,
        };

        let text = render_comparable_metadata_report_text(&report);
        assert!(text.contains("file: adminhtml.php"));
        assert!(text.contains("severity_score: 123"));
        assert!(text.contains("top_sections:"));
        assert!(text.contains("top_type_pairs:"));
        assert!(text.contains("suggested_fix_categories:"));
        assert!(text.contains("high_risk_samples:"));
        assert!(text.contains("missing_paths_sample:"));
        assert!(text.contains("extra_paths_sample:"));
        assert!(text.contains("value_mismatches_sample:"));
    }

    #[test]
    fn setup_runtime_overrides_write_expected_setup_arguments() {
        let root = std::env::temp_dir().join(format!(
            "fast-di-compile-setup-override-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));

        std::fs::create_dir_all(root.join("vendor/magento/framework")).expect("framework dir");
        std::fs::create_dir_all(root.join("vendor/magento/framework-amqp"))
            .expect("framework-amqp dir");
        std::fs::create_dir_all(root.join("vendor/acme/module-a")).expect("module-a dir");
        std::fs::create_dir_all(root.join("app/code/Acme/ModuleB")).expect("module-b dir");

        let module_paths = vec![
            root.join("vendor/acme/module-a"),
            root.join("app/code/Acme/ModuleB"),
        ];

        let mut di_config = DiConfig::default();
        apply_setup_di_compile_runtime_overrides(&mut di_config, &root, &module_paths, "php");

        let scanner = di_config
            .type_configs
            .get("Magento\\Setup\\Module\\Di\\Code\\Reader\\ClassesScanner")
            .expect("classes scanner type config");
        let exclude_patterns = scanner
            .arguments
            .iter()
            .find(|arg| matches!(arg, Argument::Array { name, .. } if name == "excludePatterns"))
            .expect("excludePatterns argument");
        let groups: FxHashMap<_, _> = match exclude_patterns {
            Argument::Array { items, .. } => items
                .iter()
                .map(|item| match item {
                    Argument::Array { name, items, .. } => (name.as_str(), items),
                    other => panic!("expected grouped array item, got {other:?}"),
                })
                .collect(),
            other => panic!("expected array argument, got {other:?}"),
        };

        let application = groups
            .get("application")
            .expect("application exclude patterns");
        let application_values: Vec<&str> = application
            .iter()
            .map(|item| match item {
                Argument::String { value, .. } => value.as_str(),
                other => panic!("expected string pattern, got {other:?}"),
            })
            .collect();
        assert!(application_values.iter().any(|p| p.contains("module-a")));
        assert!(application_values.iter().any(|p| p.contains("ModuleB")));

        let framework = groups.get("framework").expect("framework exclude patterns");
        let framework_values: Vec<&str> = framework
            .iter()
            .map(|item| match item {
                Argument::String { value, .. } => value.as_str(),
                other => panic!("expected string pattern, got {other:?}"),
            })
            .collect();
        assert!(framework_values
            .iter()
            .any(|p| p.contains("framework\\-amqp")));

        let setup = groups.get("setup").expect("setup exclude patterns");
        let setup_values: Vec<&str> = setup
            .iter()
            .map(|item| match item {
                Argument::String { value, .. } => value.as_str(),
                other => panic!("expected string pattern, got {other:?}"),
            })
            .collect();
        assert!(setup_values.iter().any(|p| p.contains("/setup")));

        let modification_chain = di_config
            .type_configs
            .get("Magento\\Setup\\Module\\Di\\Compiler\\Config\\ModificationChain")
            .expect("modification chain type config");
        let modifications_list = modification_chain
            .arguments
            .iter()
            .find(|arg| matches!(arg, Argument::Array { name, .. } if name == "modificationsList"))
            .expect("modificationsList argument");
        match modifications_list {
            Argument::Array { items, .. } => assert_eq!(items.len(), 4),
            other => panic!("expected array argument, got {other:?}"),
        }

        let plugin_list = di_config
            .type_configs
            .get("Magento\\Setup\\Module\\Di\\Code\\Generator\\PluginList")
            .expect("setup plugin list config");
        let cache_arg = plugin_list
            .arguments
            .iter()
            .find_map(|arg| match arg {
                Argument::Object { name, value, .. } if name == "cache" => Some(value.as_str()),
                _ => None,
            })
            .expect("cache object argument");
        assert_eq!(
            cache_arg,
            "Magento\\Framework\\App\\Interception\\Cache\\CompiledConfig"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn canonicalize_instance_reference_case_updates_instances_in_nested_values() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Magento\\Framework\\Filesystem\\Directory\\ReadFactory".to_string(),
            class_info(
                "Magento\\Framework\\Filesystem\\Directory\\ReadFactory",
                None,
                false,
            ),
        );

        let mut args_map: FxHashMap<String, Vec<ResolvedArg>> = FxHashMap::default();
        args_map.insert(
            "Foo\\Type".to_string(),
            vec![
                ResolvedArg {
                    name: "readFactory".to_string(),
                    resolved: ResolvedArgValue::SharedInstance(
                        "Magento\\Framework\\FileSystem\\Directory\\ReadFactory".to_string(),
                    ),
                },
                ResolvedArg {
                    name: "shape".to_string(),
                    resolved: ResolvedArgValue::PlainArray(vec![ResolvedArrayItem {
                        name: "instance".to_string(),
                        value: ResolvedArrayValue::Scalar(ResolvedScalar::String(
                            "Magento\\Framework\\FileSystem\\Directory\\ReadFactory".to_string(),
                        )),
                    }]),
                },
            ],
        );

        let case_index = build_case_index(&class_map);
        apply_case_index(&mut args_map, &case_index);

        let args = args_map.get("Foo\\Type").expect("canonicalized args");
        assert!(matches!(
            &args[0].resolved,
            ResolvedArgValue::SharedInstance(v)
            if v == "Magento\\Framework\\Filesystem\\Directory\\ReadFactory"
        ));
        assert!(matches!(
            &args[1].resolved,
            ResolvedArgValue::PlainArray(items)
                if matches!(
                    &items[0].value,
                    ResolvedArrayValue::Scalar(ResolvedScalar::String(v))
                    if v == "Magento\\Framework\\Filesystem\\Directory\\ReadFactory"
                )
        ));
    }

    // -----------------------------------------------------------------
    // build_interception_registry: interface-propagation tests
    // -----------------------------------------------------------------

    /// Helper: build a minimal registry call with no VTs, proxies, or proxy_deferred.
    fn registry(
        type_names: &[&str],
        interceptors: &[&str],
        class_map: &FxHashMap<String, ClassInfo>,
    ) -> FxHashMap<String, bool> {
        let specs: Vec<di_resolver::InterceptorSpec> = interceptors
            .iter()
            .map(|fqcn| di_resolver::InterceptorSpec {
                fqcn: fqcn.to_string(),
                plugins: vec![],
                public_methods: vec![],
            })
            .collect();
        let names: Vec<String> = type_names.iter().map(|s| s.to_string()).collect();
        build_interception_registry(
            &names,
            &specs,
            &[],
            &[],
            &DiConfig::default(),
            class_map,
        )
    }

    #[test]
    fn interface_propagation_marks_implementor_intercepted() {
        // ActionInterface is directly intercepted (plugin owner).
        // ConcreteAction implements ActionInterface → must be marked true.
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Magento\\Framework\\App\\ActionInterface".to_string(),
            class_info("Magento\\Framework\\App\\ActionInterface", None, false),
        );
        class_map.insert(
            "Magento\\Framework\\App\\Action\\AbstractAction".to_string(),
            class_info_with_implements(
                "Magento\\Framework\\App\\Action\\AbstractAction",
                None,
                vec!["Magento\\Framework\\App\\ActionInterface"],
            ),
        );

        let type_names = [
            "Magento\\Framework\\App\\ActionInterface",
            "Magento\\Framework\\App\\Action\\AbstractAction",
        ];
        let map = registry(&type_names, &["Magento\\Framework\\App\\ActionInterface"], &class_map);

        assert_eq!(map.get("Magento\\Framework\\App\\ActionInterface"), Some(&true));
        assert_eq!(
            map.get("Magento\\Framework\\App\\Action\\AbstractAction"),
            Some(&true),
            "implementor of intercepted interface must be marked intercepted"
        );
    }

    #[test]
    fn interface_propagation_two_levels_deep() {
        // A implements IFace, B implements A (not typical but tests transitivity via the
        // implements list). Also covers the real Magento pattern where classes implement
        // interfaces that implement other interfaces (multi-level interface chains).
        //
        // Plugin on IFace → A (implements IFace) → B (implements A) both intercepted.
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Vendor\\IFace".to_string(),
            class_info("Vendor\\IFace", None, false),
        );
        class_map.insert(
            "Vendor\\A".to_string(),
            class_info_with_implements("Vendor\\A", None, vec!["Vendor\\IFace"]),
        );
        class_map.insert(
            "Vendor\\B".to_string(),
            class_info_with_implements("Vendor\\B", None, vec!["Vendor\\A"]),
        );
        class_map.insert(
            "Vendor\\Unrelated".to_string(),
            class_info("Vendor\\Unrelated", None, false),
        );

        let type_names = [
            "Vendor\\IFace",
            "Vendor\\A",
            "Vendor\\B",
            "Vendor\\Unrelated",
        ];
        let map = registry(&type_names, &["Vendor\\IFace"], &class_map);

        assert_eq!(map.get("Vendor\\IFace"), Some(&true));
        assert_eq!(map.get("Vendor\\A"), Some(&true));
        assert_eq!(map.get("Vendor\\B"), Some(&true), "transitive via implements chain");
        assert_eq!(map.get("Vendor\\Unrelated"), Some(&false));
    }

    #[test]
    fn interface_propagation_does_not_affect_unrelated_classes() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Vendor\\Intercepted".to_string(),
            class_info("Vendor\\Intercepted", None, false),
        );
        class_map.insert(
            "Vendor\\Unrelated".to_string(),
            class_info("Vendor\\Unrelated", None, false),
        );
        class_map.insert(
            "Vendor\\AlsoUnrelated".to_string(),
            class_info_with_implements(
                "Vendor\\AlsoUnrelated",
                None,
                vec!["Vendor\\SomeOtherInterface"],
            ),
        );

        let type_names = [
            "Vendor\\Intercepted",
            "Vendor\\Unrelated",
            "Vendor\\AlsoUnrelated",
        ];
        let map = registry(&type_names, &["Vendor\\Intercepted"], &class_map);

        assert_eq!(map.get("Vendor\\Intercepted"), Some(&true));
        assert_eq!(map.get("Vendor\\Unrelated"), Some(&false));
        assert_eq!(
            map.get("Vendor\\AlsoUnrelated"),
            Some(&false),
            "implementing an unrelated interface must not cause interception"
        );
    }

    #[test]
    fn interface_propagation_with_leading_backslash_in_implements() {
        // PHP class files often declare implements with a leading backslash.
        // The registry must normalize these when matching against intercepted_targets.
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Vendor\\ActionInterface".to_string(),
            class_info("Vendor\\ActionInterface", None, false),
        );
        class_map.insert(
            "Vendor\\ConcreteAction".to_string(),
            class_info_with_implements(
                "Vendor\\ConcreteAction",
                None,
                vec!["\\Vendor\\ActionInterface"], // leading backslash
            ),
        );

        let type_names = ["Vendor\\ActionInterface", "Vendor\\ConcreteAction"];
        let map = registry(&type_names, &["Vendor\\ActionInterface"], &class_map);

        assert_eq!(
            map.get("Vendor\\ConcreteAction"),
            Some(&true),
            "leading backslash in implements list must be normalized"
        );
    }

    #[test]
    fn interface_propagation_class_implementing_multiple_interfaces_one_intercepted() {
        let mut class_map = FxHashMap::default();
        class_map.insert(
            "Vendor\\InterceptedInterface".to_string(),
            class_info("Vendor\\InterceptedInterface", None, false),
        );
        class_map.insert(
            "Vendor\\OtherInterface".to_string(),
            class_info("Vendor\\OtherInterface", None, false),
        );
        class_map.insert(
            "Vendor\\MultiImpl".to_string(),
            class_info_with_implements(
                "Vendor\\MultiImpl",
                None,
                vec!["Vendor\\OtherInterface", "Vendor\\InterceptedInterface"],
            ),
        );

        let type_names = [
            "Vendor\\InterceptedInterface",
            "Vendor\\OtherInterface",
            "Vendor\\MultiImpl",
        ];
        let map = registry(&type_names, &["Vendor\\InterceptedInterface"], &class_map);

        assert_eq!(map.get("Vendor\\MultiImpl"), Some(&true));
        assert_eq!(map.get("Vendor\\OtherInterface"), Some(&false));
    }

    // =========================================================================
    // IncrementalCache tests
    // =========================================================================

    #[test]
    fn incremental_cache_hash_of_nonexistent_file_returns_none() {
        assert!(
            super::IncrementalCache::hash_of(std::path::Path::new(
                "/nonexistent/__no_such_file__.php"
            ))
            .is_none()
        );
    }

    #[test]
    fn incremental_cache_hash_of_existing_file_is_stable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("a.php");
        std::fs::write(&path, b"<?php echo 1;").unwrap();
        let h1 = super::IncrementalCache::hash_of(&path).unwrap();
        let h2 = super::IncrementalCache::hash_of(&path).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn incremental_cache_hash_changes_when_file_content_changes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("b.php");
        std::fs::write(&path, b"<?php $a = 1;").unwrap();
        let h1 = super::IncrementalCache::hash_of(&path).unwrap();
        std::fs::write(&path, b"<?php $a = 2;").unwrap();
        let h2 = super::IncrementalCache::hash_of(&path).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn incremental_cache_record_updates_on_file_change() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("c.php");
        std::fs::write(&path, b"<?php $a = 1;").unwrap();
        let mut cache = super::IncrementalCache::default();
        cache.record(&path);
        let key = path.to_string_lossy().to_string();
        let hash_before = cache.files[&key].clone();

        std::fs::write(&path, b"<?php $a = 2;").unwrap();
        cache.record(&path);
        let hash_after = cache.files[&key].clone();

        assert_ne!(hash_before, hash_after);
    }

    #[test]
    fn incremental_cache_save_and_load_round_trip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let file_path = tmp.path().join("d.php");
        std::fs::write(&file_path, b"<?php // test").unwrap();

        let mut cache = super::IncrementalCache::default();
        cache.record(&file_path);

        let cache_path = tmp.path().join("cache.json");
        cache.save(&cache_path);

        let loaded = super::IncrementalCache::load(&cache_path);
        let key = file_path.to_string_lossy().to_string();
        assert!(loaded.files.contains_key(&key));
        assert_eq!(loaded.files[&key], cache.files[&key]);
    }

    #[test]
    fn incremental_cache_load_returns_empty_for_missing_file() {
        let cache =
            super::IncrementalCache::load(std::path::Path::new("/no/such/cache.json"));
        assert!(cache.files.is_empty());
    }

    // =========================================================================
    // enrich_all_constructors_with_reflection tests
    //
    // PHP_WORKER_POOL is not initialized in tests, so reflect_constructor_params
    // always returns None. All tests verify candidate-selection logic and that
    // the function handles gracefully, returning correct zero counts.
    // =========================================================================

    use php_extractor::types::{Constructor, ConstructorParam};

    fn ctor_with_const_default() -> Constructor {
        Constructor {
            params: vec![ConstructorParam {
                name: "x".to_string(),
                type_hint: None,
                is_optional: true,
                default_value: Some("Magento\\Module\\Model::SOME_CONST".to_string()),
                is_primitive: false,
                is_variadic: false,
                is_promoted: false,
            }],
        }
    }

    fn ctor_plain() -> Constructor {
        Constructor {
            params: vec![ConstructorParam {
                name: "y".to_string(),
                type_hint: Some("string".to_string()),
                is_optional: false,
                default_value: None,
                is_primitive: true,
                is_variadic: false,
                is_promoted: false,
            }],
        }
    }

    #[test]
    fn enrich_all_constructors_empty_class_map_returns_zeros() {
        let mut class_map: FxHashMap<String, ClassInfo> = FxHashMap::default();
        let (c0, c1, c2) = super::enrich_all_constructors_with_reflection(
            &mut class_map,
            &[],
            &DiConfig::default(),
            std::path::Path::new("/nonexistent"),
            "nonexistent_php_binary",
        );
        assert_eq!((c0, c1, c2), (0, 0, 0));
    }

    #[test]
    fn enrich_all_constructors_class_without_const_default_is_not_kind0_candidate() {
        // A class with a plain (non-constant) constructor is NOT a kind-0 candidate.
        // Verify function returns (0,0,0) and does not panic.
        let mut plain = class_info("Foo\\Plain", None, false);
        plain.constructor = Some(ctor_plain()); // no "::" in any default

        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\Plain".to_string(), plain);

        let (c0, c1, c2) = super::enrich_all_constructors_with_reflection(
            &mut class_map,
            &[],
            &DiConfig::default(),
            std::path::Path::new("/nonexistent"),
            "nonexistent_php_binary",
        );
        assert_eq!((c0, c1, c2), (0, 0, 0));
        // Constructor left unchanged — reflection was never triggered
        assert!(class_map["Foo\\Plain"].constructor.is_some());
    }

    #[test]
    fn enrich_all_constructors_kind0_candidate_has_const_default() {
        // Class with "::" in a default value IS a kind-0 candidate.
        // PHP pool not running → reflection returns None → count stays 0, no panic.
        let mut with_const = class_info("Foo\\WithConst", None, false);
        with_const.constructor = Some(ctor_with_const_default());

        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\WithConst".to_string(), with_const);

        let (c0, c1, c2) = super::enrich_all_constructors_with_reflection(
            &mut class_map,
            &[],
            &DiConfig::default(),
            std::path::Path::new("/nonexistent"),
            "nonexistent_php_binary",
        );
        // Candidate found but PHP unavailable → reflection fails → 0 reflected
        assert_eq!((c0, c1, c2), (0, 0, 0));
    }

    #[test]
    fn enrich_all_constructors_kind1_requires_no_ctor_and_extends() {
        // kind-1 candidates: no constructor, non-abstract, extends something.
        // Classes that DON'T qualify should not trigger reflection.
        let no_extends = class_info("Foo\\NoExtends", None, false); // no extends → NOT candidate
        let has_ctor = {
            let mut c = class_info("Foo\\HasCtor", Some("Base"), false);
            c.constructor = Some(ctor_plain()); // already has ctor → NOT candidate
            c
        };
        let is_abstract = class_info_impl("Foo\\Abstract", Some("Base"), true, vec![]); // abstract → NOT

        let with_extends = class_info("Foo\\WithExtends", Some("Base"), false); // qualifies

        let mut class_map = FxHashMap::default();
        class_map.insert("Foo\\NoExtends".to_string(), no_extends);
        class_map.insert("Foo\\HasCtor".to_string(), has_ctor);
        class_map.insert("Foo\\Abstract".to_string(), is_abstract);
        class_map.insert("Foo\\WithExtends".to_string(), with_extends);

        let type_names = vec![
            "Foo\\NoExtends".to_string(),
            "Foo\\HasCtor".to_string(),
            "Foo\\Abstract".to_string(),
            "Foo\\WithExtends".to_string(),
        ];

        let (c0, c1, c2) = super::enrich_all_constructors_with_reflection(
            &mut class_map,
            &type_names,
            &DiConfig::default(),
            std::path::Path::new("/nonexistent"),
            "nonexistent_php_binary",
        );
        assert_eq!((c0, c1, c2), (0, 0, 0));
        // Only "Foo\\WithExtends" would have been a candidate; constructor remains None
        assert!(class_map["Foo\\WithExtends"].constructor.is_none());
    }

    #[test]
    fn enrich_all_constructors_kind2_requires_vt_target_absent_from_class_map() {
        // kind-2: VT target that doesn't exist in class_map.
        // "Third\\Party\\Lib" is the concrete target of "VendorVt" but is NOT in class_map.
        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "VendorVt".to_string(),
            VirtualType {
                name: "VendorVt".to_string(),
                type_name: "Third\\Party\\Lib".to_string(),
            },
        );

        let mut class_map: FxHashMap<String, ClassInfo> = FxHashMap::default();
        // "Third\\Party\\Lib" intentionally absent

        let type_names = vec!["VendorVt".to_string()];

        let (c0, c1, c2) = super::enrich_all_constructors_with_reflection(
            &mut class_map,
            &type_names,
            &di_config,
            std::path::Path::new("/nonexistent"),
            "nonexistent_php_binary",
        );
        // Candidate exists but PHP unavailable → 0 reflected
        assert_eq!((c0, c1, c2), (0, 0, 0));
        // class_map not mutated (insertion only happens on successful reflection)
        assert!(!class_map.contains_key("Third\\Party\\Lib"));
    }

    #[test]
    fn enrich_all_constructors_vt_target_in_class_map_is_not_kind2_candidate() {
        // If the VT target already exists in class_map, it is NOT a kind-2 candidate.
        let mut di_config = DiConfig::default();
        di_config.virtual_types.insert(
            "VendorVt".to_string(),
            VirtualType {
                name: "VendorVt".to_string(),
                type_name: "Vendor\\ConcreteClass".to_string(),
            },
        );

        let mut class_map: FxHashMap<String, ClassInfo> = FxHashMap::default();
        class_map.insert(
            "Vendor\\ConcreteClass".to_string(),
            class_info("Vendor\\ConcreteClass", None, false),
        );

        let type_names = vec!["VendorVt".to_string()];
        let (c0, c1, c2) = super::enrich_all_constructors_with_reflection(
            &mut class_map,
            &type_names,
            &di_config,
            std::path::Path::new("/nonexistent"),
            "nonexistent_php_binary",
        );
        // No kind-2 candidate — class_map already has the target
        assert_eq!((c0, c1, c2), (0, 0, 0));
    }
}

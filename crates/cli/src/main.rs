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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use code_generator::{
    factory_path, generate_area_config, generate_factory, generate_interceptor, generate_proxy,
    interceptor_path, proxy_path, serialize_interception_php, write_if_changed, AREAS,
};
use di_resolver::{
    detect_factories_from_configs, detect_interceptors, detect_proxies_from_configs_with_existing,
    resolve_all_arguments,
};
use di_xml_reader::{
    find_all_di_xml_files, find_di_xml_files, find_di_xml_files_for_area, merge_configs, merge_into,
    parse_di_xml, Argument, DiConfig,
};
use php_extractor::{
    extract_file,
    types::{ClassInfo, ExtractResult},
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
}

// ---------------------------------------------------------------------------
// Incremental cache (TKT-026)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default)]
struct IncrementalCache {
    /// Map of absolute path → blake3 hex hash of that file at last compile
    files: HashMap<String, String>,
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

    fn is_unchanged(&self, path: &Path) -> bool {
        let key = path.to_string_lossy().to_string();
        let Some(cached) = self.files.get(&key) else {
            return false;
        };
        let Some(current) = Self::hash_of(path) else {
            return false;
        };
        *cached == current
    }

    fn record(&mut self, path: &Path) {
        if let Some(hash) = Self::hash_of(path) {
            self.files.insert(path.to_string_lossy().to_string(), hash);
        }
    }
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

    log::info!(
        "fast-di-compile starting\n  magento_root: {}\n  output:       {}",
        magento_root.display(),
        generated_root.display()
    );

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
    let module_paths = read_module_paths(&magento_root);
    let php_files = walk_php_files(&module_paths);
    log::info!("Found {} PHP files", php_files.len());

    let pb = progress_bar(php_files.len() as u64, "Extracting PHP classes");

    let class_map: Arc<Mutex<HashMap<String, ClassInfo>>> = Arc::new(Mutex::new(HashMap::new()));
    let fallback_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let failure_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Share cache for read access in parallel section
    let cache_ref = &cache;

    php_files.par_iter().for_each(|path| {
        let result = extract_file(path);
        pb.inc(1);
        match result {
            ExtractResult::Ok(info) => {
                let mut map = class_map.lock().unwrap();
                map.insert(info.fqcn.clone(), info);
            }
            ExtractResult::NoClass => {}
            ExtractResult::PhpFallbackFailed(e) => {
                fallback_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                log::warn!("Fallback failed for {}: {}", path.display(), e);
            }
            ExtractResult::LexError(e) => {
                failure_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                log::warn!("Lex error for {}: {e}", path.display());
            }
            ExtractResult::ParseFailure(e) => {
                failure_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                log::warn!("Parse failure for {}: {}", path.display(), e);
            }
        }
    });
    pb.finish_with_message("done");

    let class_map = Arc::try_unwrap(class_map).unwrap().into_inner().unwrap();
    let fallbacks = fallback_count.load(std::sync::atomic::Ordering::Relaxed);
    let failures = failure_count.load(std::sync::atomic::Ordering::Relaxed);
    log::info!(
        "Extracted {} classes  ({} fallbacks, {} failures)",
        class_map.len(),
        fallbacks,
        failures
    );

    // -----------------------------------------------------------------------
    // Phase 3a: Parse + merge global di.xml files (for per-area metadata)
    // -----------------------------------------------------------------------
    let di_xml_files = find_di_xml_files(&magento_root);
    log::info!("Found {} di.xml files (global)", di_xml_files.len());

    let pb = progress_bar(di_xml_files.len() as u64, "Parsing di.xml (global)");
    let global_di_configs: Vec<_> = di_xml_files
        .par_iter()
        .filter_map(|path| {
            if args.incremental && cache_ref.is_unchanged(path) {
                pb.inc(1);
                return None;
            }
            let r = parse_di_xml(path);
            pb.inc(1);
            match r {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    log::warn!("di.xml parse error {}: {e}", path.display());
                    None
                }
            }
        })
        .collect();
    pb.finish_with_message("done");

    let di_config = merge_configs(global_di_configs.clone());

    // -----------------------------------------------------------------------
    // Phase 3b: Parse + merge ALL di.xml files (all areas) for detection
    //
    // Interceptor/factory/proxy detection must consider plugins registered in
    // area-specific di.xml files (e.g. etc/adminhtml/di.xml), not just global.
    // -----------------------------------------------------------------------
    let all_di_xml_files = find_all_di_xml_files(&magento_root);
    log::info!("Found {} di.xml files (all areas)", all_di_xml_files.len());

    // Only parse files not already in the global set
    let extra_di_files: Vec<_> = all_di_xml_files
        .iter()
        .filter(|p| !di_xml_files.contains(p))
        .collect();

    let extra_configs: Vec<_> = extra_di_files
        .par_iter()
        .filter_map(|path| parse_di_xml(path).ok())
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

    // -----------------------------------------------------------------------
    // Phase 4: Detection (uses full_di_config = all areas merged)
    // -----------------------------------------------------------------------
    let composer_autoload = ComposerAutoloadIndex::from_magento_root(&magento_root);
    let proxy_targets = collect_proxy_targets_from_di_configs(&scanner_di_configs);
    let mut extra_existing_proxy_targets: HashSet<String> = HashSet::new();
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

    let interceptors = detect_interceptors(&class_map, &full_di_config);
    let factories = detect_factories_from_configs(&class_map, &full_di_config, &scanner_di_configs);
    let proxies = detect_proxies_from_configs_with_existing(
        &class_map,
        &scanner_di_configs,
        &extra_existing_proxy_targets,
    );
    log::info!(
        "Detected: {} interceptors, {} factories, {} proxies",
        interceptors.len(),
        factories.len(),
        proxies.len()
    );

    // -----------------------------------------------------------------------
    // Phase 5: Resolve arguments (global config for per-area override later)
    // -----------------------------------------------------------------------
    let args_map = resolve_all_arguments(&class_map, &di_config); // global only; per-area overrides applied later
    log::info!("Resolved arguments for {} classes", args_map.len());

    // Build all_fqcns map for interception.php (all FQCNs → bool intercepted)
    let intercepted_set: std::collections::HashSet<&str> =
        interceptors.iter().map(|s| s.fqcn.as_str()).collect();
    let all_fqcns: HashMap<String, bool> = class_map
        .keys()
        .map(|fqcn| {
            let intercepted = intercepted_set.contains(fqcn.as_str());
            (fqcn.clone(), intercepted)
        })
        .collect();

    if args.dry_run {
        log::info!("Dry run — skipping file writes");
        print_summary(&interceptors, &factories, &proxies, &args_map, &all_fqcns);
        return;
    }

    // -----------------------------------------------------------------------
    // Phase 6: Generate PHP code files (parallel)
    // -----------------------------------------------------------------------
    let pb = progress_bar(
        (interceptors.len() + factories.len() + proxies.len()) as u64,
        "Generating code",
    );
    let written = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Interceptors
    interceptors.par_iter().for_each(|spec| {
        let info = class_map.get(&spec.fqcn);
        let content = generate_interceptor(spec, info);
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

    // Proxies
    proxies.par_iter().for_each(|spec| {
        let target_info = class_map.get(&spec.target_fqcn);
        let content = generate_proxy(spec, target_info);
        let rel = proxy_path(&spec.proxy_fqcn);
        let out_path = code_root.join(&rel);
        if let Ok(changed) = write_if_changed(&out_path, &content) {
            if changed {
                written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });
    pb.finish_with_message("done");

    // -----------------------------------------------------------------------
    // Phase 7: Generate metadata files
    // -----------------------------------------------------------------------
    log::info!("Generating metadata files");

    // interception.php
    let interception_content = serialize_interception_php(&all_fqcns);
    let interception_path = metadata_root.join("interception.php");
    let _ = write_if_changed(&interception_path, &interception_content);

    // Per-area config files — each area merges global + area-specific di.xml overlays.
    let pb_area = progress_bar(AREAS.len() as u64, "Generating area configs");
    for area in AREAS {
        let area_di_files = find_di_xml_files_for_area(&magento_root, area);

        // Only re-merge if there are area-specific files beyond the global set
        let area_di_config = if area_di_files.len() > di_xml_files.len() {
            // Parse only the incremental area-specific files, then merge on top
            let area_only: Vec<_> = area_di_files
                .iter()
                .filter(|p| !di_xml_files.contains(p))
                .collect();
            if area_only.is_empty() {
                di_config.clone()
            } else {
                let extra_configs: Vec<_> = area_only
                    .iter()
                    .filter_map(|p| parse_di_xml(p).ok())
                    .collect();
                let mut merged_area = di_config.clone();
                let overlay = merge_configs(extra_configs);
                merge_into(&mut merged_area, overlay);
                merged_area
            }
        } else {
            di_config.clone()
        };

        let area_args = resolve_all_arguments(&class_map, &area_di_config);
        let area_content = generate_area_config(&area_args, &area_di_config);
        let area_path = metadata_root.join(format!("{}.php", area));
        let _ = write_if_changed(&area_path, &area_content);
        pb_area.inc(1);
    }
    pb_area.finish_with_message("done");

    let total_written = written.load(std::sync::atomic::Ordering::Relaxed);
    log::info!(
        "Code generation complete: {} files written, {} unchanged",
        total_written,
        interceptors.len() + factories.len() + proxies.len() - total_written
    );

    // Update incremental cache
    if args.incremental {
        for path in &all_di_xml_files {
            cache.record(path);
        }
        cache.save(&cache_path);
        log::debug!("Incremental cache saved to {}", cache_path.display());
    }

    // -----------------------------------------------------------------------
    // Phase 8: Validation (optional)
    // -----------------------------------------------------------------------
    if args.validate {
        // --php-generated is required (enforced earlier in main)
        let php_gen = args.php_generated.as_deref().unwrap();
        log::info!("Validating against {}", php_gen.display());
        let result = validator::validate(php_gen, &generated_root);
        println!("{}", result.summary());
        if !result.is_clean() {
            std::process::exit(1);
        }
    }

    log::info!("fast-di-compile finished successfully");
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

fn print_summary(
    interceptors: &[di_resolver::InterceptorSpec],
    factories: &[di_resolver::FactorySpec],
    proxies: &[di_resolver::ProxySpec],
    args_map: &HashMap<String, Vec<di_resolver::ResolvedArg>>,
    all_fqcns: &HashMap<String, bool>,
) {
    println!("Dry run summary:");
    println!("  Interceptors:   {}", interceptors.len());
    println!("  Factories:      {}", factories.len());
    println!("  Proxies:        {}", proxies.len());
    println!("  Classes with resolved args: {}", args_map.len());
    println!("  Total FQCNs (for interception.php): {}", all_fqcns.len());
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
        let fqcn = fqcn.trim().trim_start_matches('\\');
        if fqcn.is_empty() {
            return false;
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
                if dir.join(format!("{rel_path}.php")).is_file() {
                    return true;
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
                if dir.join(format!("{rel_path}.php")).is_file() {
                    return true;
                }
            }
        }

        false
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

fn collect_proxy_targets_from_di_configs(di_configs: &[DiConfig]) -> HashSet<String> {
    let mut targets = HashSet::new();
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

fn collect_proxy_targets_from_args(args: &[Argument], out: &mut HashSet<String>) {
    for arg in args {
        match arg {
            Argument::Object { value, .. } => maybe_push_proxy_target(value, out),
            Argument::Array { items, .. } => collect_proxy_targets_from_args(items, out),
            _ => {}
        }
    }
}

fn maybe_push_proxy_target(candidate: &str, out: &mut HashSet<String>) {
    let candidate = candidate.trim().trim_start_matches('\\');
    if let Some(target) = candidate.strip_suffix("\\Proxy") {
        if !target.is_empty() {
            out.insert(target.to_string());
        }
    }
}

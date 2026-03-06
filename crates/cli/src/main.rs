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
use std::process::Command;
use std::sync::{Arc, Mutex};

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use quick_xml::events::Event;
use quick_xml::Reader;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use code_generator::{
    extension_path, factory_path, generate_app_action_list_php, generate_area_config,
    generate_extension, generate_extension_interface, generate_factory, generate_interceptor,
    generate_plugin_list_php, generate_proxy, generate_proxy_deferred, generate_search_results,
    interceptor_path, proxy_deferred_path, proxy_path, search_results_path,
    serialize_interception_php, write_if_changed, ExtensionAttributeSpec, ExtensionSpec, AREAS,
};
use di_resolver::{
    detect_factories_from_configs, detect_interceptors, detect_proxies_from_configs_with_existing,
    resolve_all_arguments, FactorySpec,
};
use di_xml_reader::{
    find_all_di_xml_files, find_di_xml_files, find_di_xml_files_for_area, merge_configs,
    merge_into, parse_di_xml, Argument, DiConfig, Plugin,
};
use php_extractor::{
    extract_file,
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
    // Phase 3c: Collect extension-attributes metadata (for Extension* artifacts)
    // -----------------------------------------------------------------------
    let extension_attr_files = find_extension_attributes_files(&magento_root, &module_paths);
    let extension_attr_map = parse_extension_attributes_files(&extension_attr_files);
    let extension_specs = collect_extension_specs(&class_map, &extension_attr_map);

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

    let mut factories =
        detect_factories_from_configs(&class_map, &full_di_config, &scanner_di_configs, &di_config);
    let proxies = detect_proxies_from_configs_with_existing(
        &class_map,
        &scanner_di_configs,
        &extra_existing_proxy_targets,
    );

    // Extension class factories are generated by Magento's PHP scanner path.
    // Include them even when no constructor/XML factory trigger references them.
    let mut factory_seen: HashSet<String> = factories
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
    interception_di_config.plugins = merge_plugins_for_interception(&scanner_di_configs);

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

    // -----------------------------------------------------------------------
    // Phase 5: Resolve arguments (global config for per-area override later)
    // -----------------------------------------------------------------------
    let args_map = resolve_all_arguments(&class_map, &di_config); // global only; per-area overrides applied later
    log::info!("Resolved arguments for {} classes", args_map.len());

    // Build all_fqcns map for interception.php (all FQCNs → bool intercepted)
    let intercepted_set: std::collections::HashSet<&str> =
        interceptors.iter().map(|s| s.fqcn.as_str()).collect();
    let all_fqcns: HashMap<String, bool> = interception_class_map
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
    let reflected_ctor_params: HashMap<String, Vec<ConstructorParam>> = interceptors
        .iter()
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

    // Proxies
    proxies.par_iter().for_each(|spec| {
        let target_info =
            target_info_with_inherited_public_methods(&spec.target_fqcn, &interception_class_map);
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

    // -----------------------------------------------------------------------
    // Phase 7: Generate metadata files
    // -----------------------------------------------------------------------
    log::info!("Generating metadata files");

    // interception.php
    let interception_content = serialize_interception_php(&all_fqcns);
    let interception_path = metadata_root.join("interception.php");
    let _ = write_if_changed(&interception_path, &interception_content);

    // Per-area config files — each area merges global + area-specific di.xml overlays.
    let mut area_di_configs: HashMap<String, DiConfig> = HashMap::new();
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
        area_di_configs.insert((*area).to_string(), area_di_config);
        pb_area.inc(1);
    }
    pb_area.finish_with_message("done");

    // Scope-specific plugin-list metadata files.
    let plugin_list_class_definitions: Vec<String> =
        interceptors.iter().map(|spec| spec.fqcn.clone()).collect();
    let plugin_scopes = [
        "global",
        "adminhtml",
        "crontab",
        "frontend",
        "graphql",
        "webapi_rest",
        "webapi_soap",
    ];
    let pb_plugins = progress_bar(
        plugin_scopes.len() as u64,
        "Generating plugin-list metadata",
    );
    for scope in plugin_scopes {
        if let Some(scope_di_config) = area_di_configs.get(scope) {
            let content = generate_plugin_list_php(
                scope_di_config,
                &class_map,
                &plugin_list_class_definitions,
            );
            let cache_id = plugin_list_cache_id(scope);
            let path = metadata_root.join(format!("{}.php", cache_id));
            let _ = write_if_changed(&path, &content);
        }
        pb_plugins.inc(1);
    }
    pb_plugins.finish_with_message("done");

    // App action list metadata.
    let app_action_list = generate_app_action_list_php(&class_map);
    let app_action_list_path = metadata_root.join("app_action_list.php");
    let _ = write_if_changed(&app_action_list_path, &app_action_list);

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
        let archive_root = args
            .archive_root
            .clone()
            .unwrap_or_else(|| magento_root.join("generated"));
        let report_dir = args
            .compare_report_dir
            .clone()
            .unwrap_or_else(|| generated_root.join("diff"));
        match compare_against_archive(&generated_root, &archive_root, &report_dir) {
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
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("error: archive compare failed: {e}");
                std::process::exit(2);
            }
        }
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

fn collect_relative_files(root: &Path) -> std::io::Result<HashSet<String>> {
    let mut out = HashSet::new();
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
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
    factories: &[FactorySpec],
    composer_index: Option<&ComposerAutoloadIndex>,
) -> Vec<SearchResultsSpec> {
    let mut specs = Vec::new();
    let mut seen = HashSet::new();

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
    class_map: &HashMap<String, ClassInfo>,
    factories: &[FactorySpec],
    composer_index: Option<&ComposerAutoloadIndex>,
) -> Vec<ProxyDeferredSpec> {
    let mut specs = Vec::new();
    let mut seen = HashSet::new();

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
    class_map: &HashMap<String, ClassInfo>,
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

fn merge_plugins_for_interception(di_configs: &[DiConfig]) -> HashMap<String, Vec<Plugin>> {
    let mut merged: HashMap<String, HashMap<String, Plugin>> = HashMap::new();

    for cfg in di_configs {
        for (owner, plugins) in &cfg.plugins {
            let owner_plugins = merged.entry(owner.clone()).or_default();
            for plugin in plugins {
                match owner_plugins.get_mut(&plugin.name) {
                    None => {
                        owner_plugins.insert(plugin.name.clone(), plugin.clone());
                    }
                    Some(existing) => {
                        if existing.disabled && !plugin.disabled {
                            // Any active declaration across areas should keep interception active.
                            *existing = plugin.clone();
                        } else if !existing.disabled && plugin.disabled {
                            // Keep existing active plugin instead of disabling globally.
                        } else {
                            // Same active/disabled state: later config wins.
                            *existing = plugin.clone();
                        }
                    }
                }
            }
        }
    }

    let mut out = HashMap::new();
    for (owner, by_name) in merged {
        let mut plugins: Vec<Plugin> = by_name.into_values().collect();
        plugins.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
        out.insert(owner, plugins);
    }
    out
}

fn augment_with_composer_plugin_owner_classes(
    class_map: &mut HashMap<String, ClassInfo>,
    di_config: &DiConfig,
    composer_index: Option<&ComposerAutoloadIndex>,
) {
    let Some(index) = composer_index else {
        return;
    };

    let mut candidates: HashSet<String> = HashSet::new();
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
    class_map: &HashMap<String, ClassInfo>,
) -> Option<ClassInfo> {
    let normalized = fqcn.trim_start_matches('\\');
    let mut info = class_map.get(normalized)?.clone();
    if info.constructor.is_some() {
        return Some(info);
    }

    let mut seen: HashSet<String> = HashSet::new();
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
    class_map: &HashMap<String, ClassInfo>,
) -> Option<ClassInfo> {
    let normalized = fqcn.trim_start_matches('\\');
    let mut info = class_map.get(normalized)?.clone();
    info.public_methods = collect_public_methods_with_inheritance(normalized, class_map);
    Some(info)
}

fn collect_public_methods_with_inheritance(
    fqcn: &str,
    class_map: &HashMap<String, ClassInfo>,
) -> Vec<MethodSignature> {
    let mut methods = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut seen_types: HashSet<String> = HashSet::new();
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
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
    magento_root: &Path,
    php_bin: &str,
) {
    let mut reflection_cache: HashMap<String, Option<Vec<MethodSignature>>> = HashMap::new();
    let mut plugin_method_cache: HashMap<String, Option<HashSet<String>>> = HashMap::new();

    for spec in specs.iter_mut() {
        let needs_signature_normalization =
            interceptor_methods_need_reflection_normalization(&spec.public_methods);
        let expected = if spec.plugins.is_empty() {
            HashSet::new()
        } else {
            derive_intercepted_method_names_from_plugins(
                &spec.plugins,
                class_map,
                di_config,
                magento_root,
                php_bin,
                &mut plugin_method_cache,
            )
        };
        if spec.plugins.is_empty() && !needs_signature_normalization {
            continue;
        }
        if !spec.plugins.is_empty() && expected.is_empty() && !needs_signature_normalization {
            continue;
        }

        let current: HashSet<String> = spec.public_methods.iter().map(|m| m.name.clone()).collect();
        let missing_expected = !spec.plugins.is_empty() && !expected.is_subset(&current);
        if !missing_expected && !needs_signature_normalization {
            continue;
        }

        let reflected = reflection_cache
            .entry(spec.fqcn.clone())
            .or_insert_with(|| reflect_interceptable_methods(&spec.fqcn, magento_root, php_bin))
            .clone();

        let Some(mut reflected_methods) = reflected else {
            continue;
        };
        for method in reflected_methods.iter_mut() {
            normalize_reflected_method_signature(method, &spec.fqcn, class_map);
        }

        let reflected_by_name: HashMap<String, MethodSignature> = reflected_methods
            .iter()
            .map(|m| (m.name.clone(), m.clone()))
            .collect();

        if !missing_expected {
            // Keep resolver-selected method set/order, but normalize signatures/defaults
            // from reflection where available to match Magento output.
            spec.public_methods = spec
                .public_methods
                .iter()
                .map(|m| {
                    reflected_by_name
                        .get(&m.name)
                        .cloned()
                        .unwrap_or_else(|| m.clone())
                })
                .collect();
            continue;
        }

        let reflected_filtered: Vec<MethodSignature> = reflected_methods
            .into_iter()
            .filter(|m| expected.contains(&m.name))
            .collect();
        if !reflected_filtered.is_empty() {
            spec.public_methods = reflected_filtered;
        }
    }
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
    class_map: &HashMap<String, ClassInfo>,
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

fn normalize_reflected_type_hint(
    raw: &str,
    target_fqcn: &str,
    class_map: &HashMap<String, ClassInfo>,
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

fn derive_intercepted_method_names_from_plugins(
    plugins: &[di_resolver::PluginRef],
    class_map: &HashMap<String, ClassInfo>,
    di_config: &DiConfig,
    magento_root: &Path,
    php_bin: &str,
    plugin_method_cache: &mut HashMap<String, Option<HashSet<String>>>,
) -> HashSet<String> {
    let mut names = HashSet::new();

    for plugin in plugins {
        let resolved = di_config.get_instance_type(&plugin.type_name);
        let mut candidates = vec![
            resolved.trim_start_matches('\\').to_string(),
            plugin.type_name.trim_start_matches('\\').to_string(),
        ];
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

            let reflected = plugin_method_cache
                .entry(candidate.clone())
                .or_insert_with(|| {
                    reflect_interceptable_methods(candidate, magento_root, php_bin).map(|methods| {
                        let mut method_names = HashSet::new();
                        for method in methods {
                            if let Some(name) = plugin_method_to_intercepted(&method.name) {
                                method_names.insert(name);
                            }
                        }
                        method_names
                    })
                })
                .clone();
            if let Some(method_names) = reflected {
                names.extend(method_names);
            }
        }
    }

    names
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
    magento_root: &Path,
    php_bin: &str,
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

    let script = r#"
$root = $argv[1] ?? '';
$class = ltrim($argv[2] ?? '', '\\');
if ($root === '' || $class === '') {
    echo 'null';
    exit(0);
}
@require $root . '/vendor/autoload.php';
if (!class_exists($class) && !interface_exists($class) && !trait_exists($class)) {
    echo 'null';
    exit(0);
}
function tstr($t) {
    if ($t === null) return null;
    if ($t instanceof ReflectionNamedType) {
        $name = $t->getName();
        if ($t->allowsNull() && $name !== 'mixed' && $name !== 'null') {
            return '?' . $name;
        }
        return $name;
    }
    if ($t instanceof ReflectionUnionType) {
        $parts = [];
        foreach ($t->getTypes() as $part) {
            $parts[] = $part->getName();
        }
        return implode('|', $parts);
    }
    if ($t instanceof ReflectionIntersectionType) {
        $parts = [];
        foreach ($t->getTypes() as $part) {
            $parts[] = $part->getName();
        }
        return implode('&', $parts);
    }
    return null;
}
$ref = new ReflectionClass($class);
$rows = [];
foreach ($ref->getMethods(ReflectionMethod::IS_PUBLIC) as $m) {
    if ($m->isConstructor() || $m->isFinal() || $m->isStatic() || $m->isDestructor()) {
        continue;
    }
    $name = $m->getName();
    if (in_array($name, ['__sleep', '__wakeup', '__clone', '_resetState'], true)) {
        continue;
    }
    $params = [];
    foreach ($m->getParameters() as $p) {
        $defaultValue = null;
        $hasDefault = $p->isDefaultValueAvailable() && !$p->isVariadic();
        if ($hasDefault) {
            $defaultValue = var_export($p->getDefaultValue(), true);
        }
        $params[] = [
            'name' => $p->getName(),
            'type_hint' => tstr($p->getType()),
            'has_default' => $hasDefault,
            'default_value' => $defaultValue,
            'is_variadic' => $p->isVariadic(),
            'is_by_ref' => $p->isPassedByReference(),
        ];
    }
    $rows[] = [
        'name' => $name,
        'params' => $params,
        'return_type' => tstr($m->getReturnType()),
        'returns_reference' => $m->returnsReference(),
    ];
}
echo json_encode($rows);
"#;

    let output = Command::new(php_bin)
        .arg("-d")
        .arg("display_errors=0")
        .arg("-d")
        .arg("display_startup_errors=0")
        .arg("-r")
        .arg(script)
        .arg(magento_root)
        .arg(fqcn.trim_start_matches('\\'))
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_raw = String::from_utf8(output.stdout).ok()?;
    let json_trimmed = json_raw.trim();
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
    magento_root: &Path,
    php_bin: &str,
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

    let script = r#"
$root = $argv[1] ?? '';
$class = ltrim($argv[2] ?? '', '\\');
if ($root === '' || $class === '') {
    echo '[]';
    exit(0);
}
@require $root . '/vendor/autoload.php';
if (!class_exists($class) && !interface_exists($class) && !trait_exists($class)) {
    echo '[]';
    exit(0);
}
function tstr($t) {
    if ($t === null) return null;
    if ($t instanceof ReflectionNamedType) {
        $name = $t->getName();
        if ($t->allowsNull() && $name !== 'mixed' && $name !== 'null') {
            return '?' . $name;
        }
        return $name;
    }
    if ($t instanceof ReflectionUnionType) {
        $parts = [];
        foreach ($t->getTypes() as $part) {
            $parts[] = $part->getName();
        }
        return implode('|', $parts);
    }
    if ($t instanceof ReflectionIntersectionType) {
        $parts = [];
        foreach ($t->getTypes() as $part) {
            $parts[] = $part->getName();
        }
        return implode('&', $parts);
    }
    return null;
}
$ref = new ReflectionClass($class);
$ctor = $ref->getConstructor();
if ($ctor === null) {
    echo 'null';
    exit(0);
}
$params = [];
foreach ($ctor->getParameters() as $p) {
    $defaultValue = null;
    $hasDefault = $p->isDefaultValueAvailable() && !$p->isVariadic();
    if ($hasDefault) {
        $defaultValue = var_export($p->getDefaultValue(), true);
    }
    $params[] = [
        'name' => $p->getName(),
        'type_hint' => tstr($p->getType()),
        'has_default' => $hasDefault,
        'default_value' => $defaultValue,
        'is_variadic' => $p->isVariadic(),
    ];
}
echo json_encode($params);
"#;

    let output = Command::new(php_bin)
        .arg("-d")
        .arg("display_errors=0")
        .arg("-d")
        .arg("display_startup_errors=0")
        .arg("-r")
        .arg(script)
        .arg(magento_root)
        .arg(fqcn.trim_start_matches('\\'))
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_raw = String::from_utf8(output.stdout).ok()?;
    let json_trimmed = json_raw.trim();
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

fn plugin_list_cache_id(scope: &str) -> String {
    if scope == "global" {
        "primary|global|plugin-list".to_string()
    } else {
        format!("primary|global|{}|plugin-list", scope)
    }
}

fn find_extension_attributes_files(magento_root: &Path, module_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for module_path in module_paths {
        let file = module_path.join("etc/extension_attributes.xml");
        if file.exists() {
            files.push(file);
        }
    }
    let app_etc = magento_root.join("app/etc/extension_attributes.xml");
    if app_etc.exists() {
        files.push(app_etc);
    }
    files.sort();
    files.dedup();
    files
}

fn parse_extension_attributes_files(
    files: &[PathBuf],
) -> HashMap<String, Vec<ExtensionAttributeSpec>> {
    let mut out: HashMap<String, Vec<ExtensionAttributeSpec>> = HashMap::new();
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
    out: &mut HashMap<String, Vec<ExtensionAttributeSpec>>,
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
    let mut seen = HashSet::new();
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
    class_map: &HashMap<String, ClassInfo>,
    xml_attrs: &HashMap<String, Vec<ExtensionAttributeSpec>>,
) -> Vec<ExtensionSpec> {
    let mut specs_by_interface: HashMap<String, ExtensionSpec> = HashMap::new();

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

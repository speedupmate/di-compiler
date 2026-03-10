# Performance Hardening Report: rust/di-compiler

## Executive Summary

As of **March 10, 2026**, the compiler had two distinct operating modes:

- **Compile mode (without archive compare)**: ~`7.7s` (pre-optimization baseline)
- **Validation mode (`--compare-archive`)**: ~`23.9s` (pre-optimization baseline)

**Round 2 optimizations landed (TKT-056 through TKT-060):**

- FxHashMap/FxHashSet across all crates (A1)
- Lock-free class extraction + Arc\<DiConfig\> area loop (A2, A3)
- Merged three sequential reflection passes into one (B)
- Incremental di.xml cache correctness fix (C1) — bug fix, not just perf
- BFS reverse-index for interface propagation (C2)
- Archive compare parallelization — sequential → par\_iter() (D)

Post-optimization timing to be measured on next full run.

Remaining work: Phase 7 metadata generation tightening is the next identified hot path
(~4.1s of 7.7s baseline). Tracked for Phase 09 continuation.

---

## Current Performance Baseline

Latest measured run (2026-03-10) using:

```bash
cargo run -p fast-di-compile -- \
  --magento-root /var/www/application \
  --output /var/www/application/generated \
  --jobs 8 \
  --compare-archive \
  --archive-root /var/www/application/generated \
  --compare-report-dir /var/www/application/generated/diff
```

```
Total: 23.911s
├── Phase 1+2 (class extraction): 1.623s
├── Phase 3a (global di parse):   0.249s
├── Phase 3b (all-area di parse): 0.131s
├── Phase 3c (const map):         0.014s
├── Phase 4 (detection):          0.737s
├── Phase 5 (arg resolution):     0.708s
├── Phase 6 (codegen):            0.155s
├── Phase 7 (metadata):           4.109s
└── Archive compare:             16.185s
```

Derived metric:

```
Core compile (without archive compare): ~7.726s
```

Current correctness state from parity validation remains stable:

- All areas: `missing=0`, `mismatches=0`, `extra=40`
- Archive summary: `code_missing=0`, `code_extra=3`, `metadata_changed=16`

---

## 1. PHP Reflection Optimization

### 1.1 Current State

The codebase already implements a **persistent PHP worker pool** (see [`crates/cli/src/main.rs:163-376`](rust/di-compiler/crates/cli/src/main.rs#L163-L376)) that:
- Spawns long-lived PHP processes
- Maintains autoload state between requests
- Provides automatic worker recycling on failure

However, there are **multiple inefficient patterns**:

### 1.2 Issue: Individual Request Overhead

**Location**: [`reflect_constructor_params()`](rust/di-compiler/crates/cli/src/main.rs#L4164-L4215), [`reflect_interceptable_methods()`](rust/di-compiler/crates/cli/src/main.rs#L4036-L4111)

**Problem**: Each call involves:
1. Serialize request → pipe write
2. PHP processes request → autoload
3. Serialize response → pipe read
4. Deserialize JSON

Phase 6 reflection calls are already parallelized via `par_iter()`. The main sequential
bottleneck is Phase 7's three sequential `enrich_*_with_reflection` passes (see §1.3).
A batch protocol would further reduce IPC latency within each pass.

**Solution: Batch Request Protocol**

Extend the PHP worker protocol to handle batched requests:

```php
// New protocol: newline-separated requests, multi-line JSON responses
// Request:  "batch:methods:FQCN1,FQCN2,FQCN3"
// Response: [{"class":"FQCN1","methods":[...]},{"class":"FQCN2","methods":[...]}]
```

**Expected Impact**: 30-60% reduction in reflection IPC overhead (lower end because
Phase 6 is already parallel; main gain is in Phase 7 sequential enrichment passes).

### 1.3 Issue: Redundant Reflection Calls

**Location**: [`enrich_interceptor_specs_with_reflection()`](rust/di-compiler/crates/cli/src/main.rs#L3535-L3667)

```rust
// Phase A: Reflect plugin FQCNs
let plugin_fqcns_to_reflect: HashSet<String> = specs.iter()
    .flat_map(|spec| spec.plugins.iter()...)
    .filter(|fqcn| !class_map.contains_key(fqcn))
    .collect();

// Phase B: Reflect interceptor targets
let specs_needing_reflection: HashSet<String> = specs.iter()
    .filter(|spec| spec_needs_reflection(...))
    .map(|spec| spec.fqcn.clone())
    .collect();
```

**Problem**: Multiple passes over the same data with potential duplicates. Same FQCN may be reflected multiple times across different specs.

**Solution**: Deduplicate and cache reflection results

```rust
// Single pass: collect all unique FQCNs needing reflection
let all_fqcns: HashSet<String> = plugin_fqcns_to_reflect
    .union(&specs_needing_reflection)
    .cloned()
    .collect();

// Reflect all unique FQCNs once, store in shared cache
let reflection_cache: HashMap<String, CachedReflection> = all_fqcns
    .par_iter()
    .filter_map(|fqcn| {
        let result = worker.request(fqcn)?;
        Some((fqcn.clone(), result))
    })
    .collect();
```

### 1.4 Issue: Reflection in Metadata Phase — Three Sequential Passes

**Location**: Phase 7 metadata generation at [`crates/cli/src/main.rs:1116-1141`](rust/di-compiler/crates/cli/src/main.rs#L1116-L1141)

```rust
// Three sequential rayon barriers:
let reflected_metadata_ctors =
    enrich_constructor_defaults_with_reflection(&mut metadata_class_map, ...);
let reflected_inherited_ctors =
    enrich_inherited_constructors_with_reflection(&mut metadata_class_map, ...);
let reflected_virtual_target_ctors =
    enrich_virtual_target_constructors_with_reflection(&mut metadata_class_map, ...);
```

**Problem**:
1. Each function scans `class_map`, collects candidates, then `par_iter()` reflects them
2. Two synchronization barriers between passes: all workers complete pass N before pass N+1 begins
3. FQCNs qualifying under multiple criteria are reflected multiple times

**Solution**:
1. Merge all three candidate-collection steps into a single scan
2. Deduplicate FQCNs across all three criteria
3. Execute one unified `par_iter()` reflection pass — eliminates two barriers

---

## 2. Memory Allocation Optimization

### 2.1 Current HashMap/HashSet Usage

The codebase uses `std::collections::HashMap` and `HashSet` extensively. These have:
- Per-entry allocation overhead
- Poor cache locality
- Hash computation on every insert/lookup

**Locations**: Throughout [`main.rs`](rust/di-compiler/crates/cli/src/main.rs)

```rust
// Heavy HashMap usage in hot paths
let class_map: Arc<Mutex<HashMap<String, ClassInfo>>> = ...;
let factories: Vec<FactorySpec> = ...;
let proxies: HashSet<String> = ...;
```

### 2.2 Issue: FQCN String Duplication

**Problem**: The same FQCN string (e.g., `"Magento\Framework\App\RequestInterface"`) is stored repeatedly:
- In HashSet for deduplication
- In Vec for iteration
- As HashMap keys
- In spec structures

Each creates a new heap allocation.

**Solution: Use `FxHashMap`/`FxHashSet`**

From [`Cargo.toml`](rust/di-compiler/Cargo.toml), the workspace uses `rustc-hash`:

```toml
# Already available
rustc-hash = "2"
```

**Action**: Replace `HashMap`/`HashSet` with `FxHashMap`/`FxHashSet` for hot paths:

```rust
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

// Before
let mut map: HashMap<String, ClassInfo> = HashMap::new();

// After
let mut map: FxHashMap<String, ClassInfo> = FxHashMap::default();
```

**Expected Impact**: 20-30% faster hashing, better memory locality

### 2.3 Issue: Arc<Mutex<HashMap>> Pattern

**Location**: Phase 1+2 at [line 451](rust/di-compiler/crates/cli/src/main.rs#L451)

```rust
let class_map: Arc<Mutex<HashMap<String, ClassInfo>>> = Arc::new(Mutex::new(HashMap::new()));

php_files.par_iter().for_each(|path| {
    // ... extraction ...
    let mut map = class_map.lock().unwrap();  // Serialized access!
    map.insert(info.fqcn.clone(), info);
});
```

**Problem**:
1. `Mutex` creates serialization bottleneck
2. Each insert requires lock acquisition
3. Parallel workers compete for lock

**Solution**: Use lock-free accumulation pattern:

```rust
let class_map: FxHashMap<String, ClassInfo> = php_files
    .par_iter()
    .filter_map(|path| {
        let result = extract_file(path);
        pb.inc(1);
        match result {
            ExtractResult::Ok(info) => Some((info.fqcn.clone(), info)),
            ExtractResult::PhpFallbackFailed(e) => {
                fallback_count.fetch_add(1, Relaxed);
                log::warn!(...);
                None
            }
            ExtractResult::LexError(e) | ExtractResult::ParseFailure(e) => {
                failure_count.fetch_add(1, Relaxed);
                log::warn!(...);
                None
            }
            ExtractResult::NoClass => None,
        }
    })
    .collect();
```

**Expected Impact**: Eliminates lock contention, improves parallelism

### 2.4 Issue: Large Vector Reallocations

**Location**: Argument resolution in [`resolve_all_arguments()`](rust/di-compiler/crates/di-resolver/src/arguments.rs)

**Problem**: `Vec::push` can trigger reallocations when capacity is exceeded.

**Solution**: Pre-allocate with estimated capacity:

```rust
// Estimate based on known class count
let estimated_count = class_map.len() * 2; // Conservative estimate
let mut args_map: FxHashMap<String, Vec<ResolvedArg>> =
    FxHashMap::with_capacity_and_hasher(estimated_count, Default::default());
```

### 2.5 Issue: DiConfig and argument_type_names Cloned Per Area

**Location**: Phase 7 area loop at [lines 1172-1301](rust/di-compiler/crates/cli/src/main.rs#L1172-L1301)

Inside `AREAS.par_iter()`, two large allocations are deep-copied for every area:

```rust
// metadata_base_di_config.clone() called up to 7 times (lines 1188, 1198, 1205)
let area_di_config = if area_only.is_empty() {
    metadata_base_di_config.clone()  // Full DiConfig deep copy
} else {
    let mut merged = metadata_base_di_config.clone();  // Another full deep copy
    // apply area-specific overlays...
    merged
};

// argument_type_names.clone() called per area (line 1243)
argument_type_names.clone()
```

**Problem**: `DiConfig` contains multiple `HashMap<String, ...>` fields with thousands of entries. Deep-copying 7 times is significant allocation pressure.

**Solution**: Wrap in `Arc<T>` before the parallel loop. Areas with no overrides clone the `Arc` (pointer copy); only areas with actual area-specific files clone the inner value:

```rust
let base_di_config = Arc::new(metadata_base_di_config);
let base_type_names = Arc::new(argument_type_names);

AREAS.par_iter().map(|&area| {
    let area_di_config: Arc<DiConfig> = if area_only.is_empty() {
        Arc::clone(&base_di_config)  // zero-copy
    } else {
        let mut merged = (*base_di_config).clone();
        // apply area-specific overlays...
        Arc::new(merged)
    };
    // ...
})
```

---

## 3. Parallel Execution Efficiency

### 3.1 Current Rayon Usage

The codebase uses `rayon` for parallelization:

```rust
php_files.par_iter().for_each(|path| { ... });
di_xml_files.par_iter().filter_map(...).collect();
interceptors.par_iter().for_each(|spec| { ... });
```

### 3.2 Issue: Synchronization Overhead in Phase 7

**Location**: Area config generation at [lines 1171-1301](rust/di-compiler/crates/cli/src/main.rs#L1171-L1301)

```rust
let area_di_configs: HashMap<String, DiConfig> = AREAS
    .par_iter()
    .map(|&area| {
        // Each area does heavy computation:
        // 1. Find di.xml files (only area-specific, not global)
        // 2. Parse area-specific di.xml files
        // 3. Clone + overlay global config
        // 4. Resolve arguments for all type names
        // 5. Generate output
        // ...
    })
    .collect();
```

**Problem**: Each area clones shared data (`metadata_base_di_config`, `argument_type_names`) and runs `resolve_all_arguments_for_named_types` independently — even though most type names resolve to identical args across areas.

**Solution**:
1. Pre-wrap shared data as `Arc<T>` to eliminate per-area deep copies (see §2.5)
2. Consider pre-computing argument resolution for types that are identical across all areas, then applying only area-specific overrides

---

## 4. Caching and Incremental Computation

### 4.1 Current Incremental Cache

**Location**: [lines 116-157](rust/di-compiler/crates/cli/src/main.rs#L116-L157)

```rust
#[derive(Serialize, Deserialize, Default)]
struct IncrementalCache {
    files: HashMap<String, String>,  // path → blake3 hash
}
```

**Features**:
- File-level change detection via Blake3 hashing
- Optional enablement via `--incremental` flag

### 4.2 Issue: Coarse-Grained Invalidation

**Problem**: If ANY file changes, large portions may be recomputed.

**Solution**: Finer-grained caching:

```rust
struct IncrementalCache {
    // File hashes
    files: FxHashMap<String, String>,

    // Per-class cache entries
    class_cache: FxHashMap<String, ClassCacheEntry>,
}

struct ClassCacheEntry {
    hash: String,
    generated_code: Option<String>,
    resolved_args: Option<Vec<ResolvedArg>>,
}
```

### 4.3 Issue: Incremental di.xml Cache is Broken (Correctness Bug)

**Location**: Phase 3 at [lines 497-525](rust/di-compiler/crates/cli/src/main.rs#L497-L525)

```rust
let global_di_path_configs: Vec<_> = di_xml_files
    .par_iter()
    .filter_map(|path| {
        if args.incremental && cache_ref.is_unchanged(path) {
            return None;  // File is DROPPED — not included in merged config
        }
        let r = parse_di_xml(path);
        // ...
    })
    .collect();
```

**Problem**: When `--incremental` is used, unchanged di.xml files are excluded from
`global_di_path_configs`, meaning their configurations are **absent from the merged
`di_config`**. This produces an incomplete and incorrect merged config for the entire
compilation — plugins, preferences, virtual types, and type arguments declared in
unchanged di.xml files are all silently dropped.

**Until this is fixed, `--incremental` must not be used in production.**

**Solution**: Store the parsed `DiConfig` per file in the cache alongside the hash,
so unchanged files can be replayed from cache:

```rust
struct IncrementalCache {
    files: FxHashMap<String, String>,         // path → blake3 hash
    di_configs: FxHashMap<String, DiConfig>,  // path → cached parsed config
}

// On cache hit: deserialize and return the cached DiConfig
// On cache miss: parse, store in cache, return parsed DiConfig
```

---

## 5. Algorithmic Improvements

### 5.1 Issue: Fixed-Point Interception Interface Propagation

**Location**: [`build_interception_registry()`](rust/di-compiler/crates/cli/src/main.rs#L2407-L2422)

```rust
// Fixed-point: iterate class_map until no new intercepted classes are added
let mut changed = true;
while changed {
    changed = false;
    for (fqcn, info) in class_map {
        if intercepted_targets.contains(fqcn.as_str()) { continue; }
        let is_intercepted_via_iface = info.implements.iter()
            .any(|iface| intercepted_targets.contains(iface.trim_start_matches('\\')));
        if is_intercepted_via_iface && intercepted_targets.insert(fqcn.clone()) {
            changed = true;
        }
    }
}
```

**Complexity**: O(n × k) where n = class_map size, k = interface hierarchy depth.
With Magento's shallow hierarchy (~3–4 levels), this is effectively O(4n) — not
quadratic in practice. However it performs k full scans of the entire class map.

**Solution**: BFS with a prebuilt reverse-index (interface → implementors). Processes
each class exactly once in O(n + edges):

```rust
// Build reverse-index once: interface/class → Vec<direct implementors>
let mut implementors: FxHashMap<&str, Vec<&str>> = FxHashMap::default();
for (fqcn, info) in class_map {
    for iface in &info.implements {
        implementors
            .entry(iface.trim_start_matches('\\'))
            .or_default()
            .push(fqcn.as_str());
    }
}

// BFS from intercepted seed nodes — O(n + edges), single pass
let mut queue: std::collections::VecDeque<&str> =
    intercepted_targets.iter().map(|s| s.as_str()).collect();
while let Some(intercepted) = queue.pop_front() {
    for &implementor in implementors.get(intercepted).into_iter().flatten() {
        if intercepted_targets.insert(implementor.to_string()) {
            queue.push_back(implementor);
        }
    }
}
```

### 5.2 Issue: Repeated DI Config Merging Per Area

**Location**: Phase 7 area configs at [lines 1182-1206](rust/di-compiler/crates/cli/src/main.rs#L1182-L1206)

```rust
// Only area-specific files (not already in global set) are re-parsed:
let area_only: Vec<_> = area_di_files
    .iter()
    .filter(|p| !di_xml_files_set.contains(p))  // global files are NOT re-parsed
    .collect();

// Fast path: no area-specific files → just clone global config
let area_di_config = if area_only.is_empty() {
    metadata_base_di_config.clone()  // real cost: DiConfig deep copy × 7
} else {
    let extra_configs: Vec<_> = area_only
        .iter()
        .filter_map(|p| parse_di_xml(p).ok())  // only area-specific files parsed
        .collect();
    let mut merged = apply_module_config_on_primary(
        metadata_base_di_config.clone(),
        merge_configs(extra_configs),
    );
    // ...
    merged
};
```

Global di.xml files are NOT re-parsed per area. The actual cost is:
- `metadata_base_di_config.clone()` up to 7 times (one per area)
- Parsing only the area-specific files (typically a small set, e.g. `adminhtml/di.xml`)

**Solution**: See §2.5 — wrap `metadata_base_di_config` in `Arc<DiConfig>` so areas
without overrides share a single allocation (zero-copy reference).

---

## 6. I/O Optimization

### 6.1 Issue: write_if_changed Hash Logic Bug (Fixed)

**Location**: [`crates/code-generator/src/writer.rs`](rust/di-compiler/crates/code-generator/src/writer.rs)

**Previous code**:
```rust
if let Ok(existing) = std::fs::read_to_string(path) {
    if existing == content {
        return Ok(false);
    }
    // Hash check runs ONLY when strings are confirmed NOT equal:
    // - As optimization: useless (strings already compared)
    // - As correctness: hazard (hash collision silently skips write)
    let mut h1 = FxHasher::default(); existing.hash(&mut h1);
    let mut h2 = FxHasher::default(); content.hash(&mut h2);
    if h1.finish() == h2.finish() {
        return Ok(false);  // BUG: could skip write on hash collision
    }
}
```

**Fixed code** (now in codebase):
```rust
if let Ok(existing) = std::fs::read_to_string(path) {
    // Hash-first: fast reject on definite difference, confirm with
    // string equality to guard against hash collisions.
    let mut h1 = FxHasher::default(); existing.hash(&mut h1);
    let mut h2 = FxHasher::default(); content.hash(&mut h2);
    if h1.finish() == h2.finish() && existing == content {
        return Ok(false);
    }
}
```

This fix: (1) uses hash as fast discriminator, (2) confirms with string equality to
be collision-safe, (3) correctly writes for changed content in all cases.

### 6.2 Issue: Sequential Metadata Normalization

**Location**: Archive compare at [lines 2566-2593](rust/di-compiler/crates/cli/src/main.rs#L2566-L2593)

```rust
// Sequential: each file spawns a PHP subprocess, blocks until done
for rel in common {
    let archive_json = normalize_metadata_to_json_bytes(&archive_src, php_bin)?;
    let output_json  = normalize_metadata_to_json_bytes(&output_src, php_bin)?;
    // ...
}
```

**Problem**: Each `normalize_metadata_to_json_bytes` call spawns a `php` subprocess
that runs `include $file` and serializes to JSON. With N metadata files, this is 2N
sequential subprocess spawns. With a measured cost of **16.2s** (68% of total validation
time), this is the dominant bottleneck in validation mode.

**Solution**: Parallelize with `par_iter()` using the existing PHP worker pool or
parallel subprocess spawning. Since normalization is read-only and per-file independent,
this is safe:

```rust
let results: Vec<_> = common
    .par_iter()
    .map(|rel| {
        let archive_json = normalize_metadata_to_json_bytes(&archive_src, php_bin)?;
        let output_json  = normalize_metadata_to_json_bytes(&output_src, php_bin)?;
        Ok((rel, archive_json, output_json))
    })
    .collect::<std::io::Result<Vec<_>>>()?;

for (rel, archive_json, output_json) in results {
    // write files, build reports...
}
```

---

## 7. Priority Recommendations

Based on impact and implementation complexity:

### High Priority (High Impact, Lower Complexity)

| # | Optimization | Est. Time Savings | Complexity |
|---|-------------|------------------|------------|
| 1 | Eliminate `Arc<Mutex<HashMap>>` contention in Phase 1+2 | 0.2-0.6s | Low |
| 2 | Reflection batching/dedup on PHP worker protocol | 0.5-1.5s | Medium |
| 3 | Phase 7 area metadata generation tightening | 0.3-0.8s | Medium |
| 4 | FxHashMap/FxHashSet conversion in hot maps/sets | 0.2-0.6s | Low |

### Medium Priority (High Impact, Higher Complexity)

| # | Optimization | Est. Time Savings | Complexity |
|---|-------------|------------------|------------|
| 5 | Fix incremental di.xml cache (correctness + perf) | blocks `--incremental` | Medium |
| 6 | Merge three metadata reflection passes into one | 0.2-0.5s | Medium |
| 7 | Arc<DiConfig> + Arc<Vec> in area loop (§2.5) | 0.1-0.2s | Low |
| 8 | Archive compare normalization parallelization | 4-10s (validation only) | Medium |

### Lower Priority (Moderate Impact, High Complexity)

| # | Optimization | Est. Time Savings | Complexity |
|---|-------------|------------------|------------|
| 9 | BFS reverse-index for interception interface propagation | minor + clarity | Medium |
| 10 | Finer-grained incremental cache (per-class artifacts) | 0.3-1.0s | High |

---

## 8. Implementation Roadmap

### Phase A: Runtime Hot Path First (1-2 days)

1. **Eliminate Arc<Mutex> in Phase 1+2**
   - Location: [`crates/cli/src/main.rs:451`](rust/di-compiler/crates/cli/src/main.rs#L451)
   - Goal: remove worker lock contention in class extraction

2. **Introduce FxHashMap/FxHashSet in confirmed hot paths**
   - Location: CLI + resolver hot structures
   - Goal: faster hashing and lower CPU overhead

3. **Arc<DiConfig> + Arc<Vec> in area loop**
   - Location: Phase 7 area config parallel section
   - Goal: eliminate DiConfig deep copies

### Phase B: Reflection + Metadata (2-3 days)

4. **Merge three sequential metadata reflection passes**
   - Location: [`crates/cli/src/main.rs:1116-1141`](rust/di-compiler/crates/cli/src/main.rs#L1116-L1141)
   - Goal: eliminate two rayon synchronization barriers in Phase 7

5. **Implement batch reflection protocol + dedup**
   - Location: [`crates/cli/src/main.rs:163-376`](rust/di-compiler/crates/cli/src/main.rs#L163-L376), [`enrich_interceptor_specs_with_reflection()`](rust/di-compiler/crates/cli/src/main.rs#L3535-L3667)
   - Goal: reduce IPC round-trips and repeated reflections

### Phase C: Correctness + Cache Hardening (2-3 days)

6. **Fix incremental di.xml cache**
   - Location: Phase 3 parsing/merge path
   - Goal: fix correctness bug where unchanged files are dropped from merge

7. **BFS reverse-index for interception interface propagation**
   - Location: [`build_interception_registry()`](rust/di-compiler/crates/cli/src/main.rs#L2407-L2422)
   - Goal: single-pass O(n + edges) propagation instead of repeated full scans

### Phase D: Advanced Optimizations (2-4 days)

8. **Archive compare parallelization**
   - Location: [`crates/cli/src/main.rs:2566-2593`](rust/di-compiler/crates/cli/src/main.rs#L2566-L2593)
   - Goal: eliminate sequential PHP subprocess spawning in validation path
   - Note: 16.2s measured cost makes this high impact for debug/validation workflows

9. **Finer-grained incremental cache (per-class artifacts)**
   - Location: `IncrementalCache` + resolve/codegen phases
   - Goal: faster repeat compiles when only a few classes change

---

## Conclusion

The project is on the right track for correctness and core compile speed, and
the runtime optimization order should stay usage-focused. Highest-priority work is:

1. **Lock-free class extraction path**
2. **FxHashMap conversion**
3. **Arc-share DiConfig across areas**
4. **Reflection batching/dedup**

With these priorities, realistic near-term targets are:

- sub-6s core compile mode on this install
- sub-10s validation mode (`--compare-archive`) after archive compare parallelization

while preserving the current parity bar (`missing=0`, `mismatches=0`).

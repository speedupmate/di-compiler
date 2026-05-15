# Phase 7 Performance — Analysis, Implementation, and Status

Date: 2026-05-15  
Status: **all planned optimizations implemented**  
Supersedes: `phase-7-optimization-plan.md`, `phase-7-superfast-patterns.md`

---

## Context

Phase 7 ("Generate metadata files") does more than emit files. At peak it ran late compiler
work inside a sequential emission loop: PHP constant resolution, generated class map
extraction, constructor reflection enrichment, 7 × full argument-resolution passes, 7 ×
config serializations, 7 × plugin-list compiles, and app-action-list generation.

**Primary bottleneck (pre-optimization):** `resolve_all_arguments_for_named_types` called 7 times
for ~25,234 types = ~176K resolutions per run. No caching, no deduplication.

**Target end state:** Phase 7 is mostly emission-only. IR finalized in earlier phases;
Phase 7 fingerprints, skips unchanged outputs, serializes only what changed.

---

## Baseline and Milestone Measurements

| Date | Total | Phase 7 | Note |
|------|-------|---------|------|
| 2026-03-10 | 23.9s | 4.1s | Round 2 baseline (TKT-056–060) |
| 2026-03-10 | 2.4s | — | Post-TKT-061, archive compare separate |
| 2026-05-15 | ~2.2s | ~0.85s | Post all Phase 7 optimizations, first run |
| 2026-05-15 | ~1.29s | ~6ms | Post all Phase 7 optimizations, FP-SCOPE-3 hit |

**Phase 7 sub-bucket breakdown (first run, 2026-05-15):**

| Bucket | Time |
|--------|------|
| PHP constant resolution | ~11ms |
| Setup runtime overrides | ~48ms |
| Generated class map extraction | ~244ms (4953 files scanned) |
| Constructor reflection enrichment | ~121ms |
| Interception registry build | ~22ms |
| Global arg resolution (par_iter) | ~37ms |
| Dep reverse index build | ~40ms |
| Area-config loop (7 areas) | ~153ms |
| Plugin-list loop (parallel) | ~11ms |

---

## Implemented Optimizations

### TIMERS — Granular per-bucket timers
`log::info!` timers around all major Phase 7 call sites; `log::debug!` inside loops.
Visible with `--verbose` (`RUST_LOG=info`) or `RUST_LOG=debug` for inner-loop granularity.

### OPT-PLUGIN — Parallelize plugin-list loop ✅
`for scope in plugin_scopes` → `par_iter().for_each()`. Gain: 44ms → 11ms.
Each scope is independent; `write_if_changed` uses atomic I/O.

### OPT-TYPENAMES — Arc<Vec<String>> for argument_type_names ✅
Eliminated 25K-item `Vec<String>` clone per area on the no-extra-VT fast path.
`Arc::clone` (refcount) instead; only areas with new VTs allocate a new vec.

### OPT-INDEXES — Incremental O(1) preference/typeconfig index updates ✅
`insert_preference` / `insert_type_config` on `DiConfig` update `preference_keys_lc` /
`type_config_keys_lc` in O(1) via `entry().or_insert_with()`. Removed full
`refresh_lookup_indexes()` calls from merger and area loop.

### OPT-DELTA + DELTA-E — Global args once + parallel resolver ✅
Single global arg resolution pass before the area loop using `par_iter` (37ms).
Per-area: compute "affected" type set from area-specific di.xml keys and interception
preference overrides; re-resolve only the affected set as a delta. Delta merges into
global baseline via `generate_area_config_with_overrides(baseline, delta, ...)`.

Gain: ~850ms → ~37ms global + ~90ms area deltas total (7 areas).

### DEP-IDX — Dep reverse index parallel ✅
Dependency reverse index (`referenced_fqcn → owners`) built in one parallel pass after
global resolution. Used to expand "affected" set when preferences change per area.
Build time: ~40ms.

### SER-1/SER-2 — `write!` into pre-allocated String ✅
`generate_area_config_with_overrides` now uses `write!(out, ...)` instead of `format!`
concatenation. Capacity estimated from entry counts (no reallocations for typical areas).

### SER-4 — `escape_php` Cow (zero-alloc for non-special strings) ✅
`escape_php` returns `Cow<'_, str>` — borrows original for strings containing no `\` or
`'`. FQCNs with backslashes use single-pass `chars()` iteration (fixed non-ASCII
corruption bug from prior `bytes()` approach).

### OVERLAY — Serializer baseline+delta split instead of HashMap clone ✅
`generate_area_config_with_overrides` accepts `(args_baseline, args_delta)` instead of
one pre-merged map. Eliminates 25K `HashMap::clone()` per area in the delta path.
Wrappers pass `&FxHashMap::default()` as delta for the simple path.

Gain: area-config loop 204ms → 153ms.

### FP-SCOPE-3 — Skip Phase 7 entirely on fingerprint match ✅
Before Phase 7 starts, hash all inputs:
- Full `ClassInfo` per entry (constructors, params, types, defaults, not just keys)
- All `DiConfig` values (argument trees, plugin sort/disabled/type, preferences, VTs)
- Interceptors/factories/proxies/search_results/proxy_deferred/extension_specs
- Resolved PHP constants (key+value pairs)
- Module paths (absolute `PathBuf` list)

Fingerprint stored at `generated/.fast-di-cache/phase7.fp`. Also verifies all expected
output files exist (7 area `.php`, `interception.php`, `app_action_list.php`, 7 plugin-list `.php`).

On match: Phase 7 entirely skipped. Gain: ~850ms → ~6ms on no-change runs.

**Fingerprint version:** 3 (bumped during review-finding fixes to invalidate stale caches).

---

## Correctness Notes

### Affected-set expansion
The delta path uses a simple "union of area-specific di.xml keys + interception preference
keys" as the affected set, expanded through the dep reverse index (`pref_consumers`).
This covers type_configs, virtual_types, and preference changes correctly. It does not
instrument the resolver for full dependency tracking (DependencyRecorder approach
from the original plan was deferred — the simple approach is correct for Magento's
DI patterns and the actual delta sizes are small enough that over-inclusion is safe).

### FxHasher determinism
`rustc_hash::FxHasher::default()` is deterministic across process runs (state initializes
to 0, fixed polynomial algorithm, no random seed). Fingerprints are stable.

### PHP constants before fingerprint
`resolve_php_constants_in_config` runs before the fingerprint check (~11ms penalty on
every run including FP hits) so that PHP runtime changes properly invalidate the cache.

---

## Deferred Items

| Item | Reason |
|------|--------|
| FP-SCOPE-1: identical area config dedup | Low value after DELTA; most areas resolve fast |
| FP-SCOPE-2: identical plugin-list scope dedup | Low value after plugin parallelization |
| SER-3: pre-sort keys in IR | Premature; sort cost is negligible vs resolution |
| SER-5: parallelize three sections within area | Not a bottleneck |
| OPT-OVERLAY via DiConfigView trait | Complexity not justified; OVERLAY via split-map is sufficient |
| IR-CACHE: disk-backed binary IR | Only helps "some files changed" path; cold compile unaffected |
| DAEMON: persistent daemon mode | Requires IPC protocol design; deferred |

---

## Critical Files

| File | Role |
|------|------|
| `crates/cli/src/main.rs` | Phase 7 orchestration, fingerprint compute + skip, delta loop |
| `crates/di-resolver/src/arguments.rs` | `resolve_all_arguments_for_named_types` (par_iter) |
| `crates/di-xml-reader/src/model.rs` | `DiConfig` + `insert_preference` / `insert_type_config` |
| `crates/di-xml-reader/src/merger.rs` | `merge_configs`, `merge_into_impl` |
| `crates/code-generator/src/area_config.rs` | `generate_area_config_with_overrides` (baseline+delta) |
| `crates/code-generator/src/plugin_list.rs` | `compile_plugin_list` |
| `crates/code-generator/src/metadata.rs` | `escape_php` (Cow, chars-based) |

---

## Verification Protocol

```bash
# Build
cd /var/www/application/rust/di-compiler && cargo build --release 2>&1 | tail -3

# Cold run (clear FP cache first)
rm -f /var/www/application/generated/.fast-di-cache/phase7.fp
chown -R magento:magento /var/www/application
su magento -c "cd /var/www/application && RUST_LOG=info \
  /var/www/application/rust/di-compiler/target/release/fast-di-compile \
  --magento-root /var/www/application" 2>&1 | grep -E "Phase|Total|skipped"

# Warm run (FP hit)
su magento -c "cd /var/www/application && RUST_LOG=info \
  /var/www/application/rust/di-compiler/target/release/fast-di-compile \
  --magento-root /var/www/application" 2>&1 | grep -E "Phase|Total|skipped"

# Unit tests
cargo test --workspace
```

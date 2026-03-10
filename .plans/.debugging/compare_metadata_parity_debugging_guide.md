# Metadata Parity Script Debugging Guide

This guide explains how to use `compare_metadata_parity.php` for fast parity triage and root-cause isolation.

## Script
- Path: `/var/www/application/rust/di-compiler/.plans/.debugging/compare_metadata_parity.php`
- Purpose: Compare truth (`generated/_metadata`) vs Rust output (`generated/metadata`) across:
  - `arguments`
  - `preferences`
  - `instanceTypes`

## Quick Start

Run all areas:

```bash
php /var/www/application/rust/di-compiler/.plans/.debugging/compare_metadata_parity.php
```

Run one area with larger samples:

```bash
php /var/www/application/rust/di-compiler/.plans/.debugging/compare_metadata_parity.php \
  --area=global \
  --max-samples=30
```

Run against custom roots:

```bash
php /var/www/application/rust/di-compiler/.plans/.debugging/compare_metadata_parity.php \
  --truth=/path/to/_metadata \
  --output=/path/to/metadata
```

## Output Semantics

For each area and section (`arguments`, `preferences`, `instanceTypes`):

- `missing`: Path exists in truth but not output.
- `extra`: Path exists in output but not truth.
- `mismatches`: Same path exists in both but normalized values differ.

Normalization behavior:
- Associative arrays are key-sorted before comparison (stable map diff).
- List arrays preserve order (order-sensitive where semantically relevant).

## Triage Flow

1. Start with `global`.
2. Fix shared/global issues first.
3. Re-run and then address area-specific deltas (`frontend`, `adminhtml`, etc.).
4. Keep changes small and rerun script after each change.

## Pattern-to-Root-Cause Map

### 1) `arguments.*.excludePatterns` missing
Likely cause:
- Missing setup runtime injection parity (from setup compile command overrides).

Check/fix:
- Runtime setup override layer in CLI before argument resolution.
- `Magento\\Setup\\Module\\Di\\Code\\Reader\\ClassesScanner.excludePatterns`.

### 2) `arguments.*OperationPool.operations.*` extra/mismatch
Likely cause:
- Argument merge semantics drift (app/etc vs module di.xml override behavior).

Check/fix:
- Array merge strategy in `di-resolver/src/arguments.rs`.
- Validate replacement vs recursive merge behavior for this key.

### 3) `arguments.*Mcrypt.*` mismatches (`MCRYPT_*` vs values)
Likely cause:
- PHP constant resolution gap.

Check/fix:
- Constant map bootstrap in CLI and fallback resolution path.

### 4) `preferences.*` missing for interception/setup classes
Likely cause:
- Interceptor preference generation gaps.

Check/fix:
- `build_interception_preferences` in `crates/cli/src/main.rs`.
- Ensure preference-chain-aware interceptor mapping where required.

### 5) `instanceTypes.*` interceptor/base-class drifts
Likely cause:
- Interception registry or preference substitution parity mismatch.

Check/fix:
- Interception registry propagation and substitution logic.
- Setup `ModificationChain` parity where relevant.

## Common Pitfalls

- Comparing only top-level `arguments` keys misses nested path drift.
- Using `isset` for key checks drops valid `null` paths; use `array_key_exists` semantics.
- Interpreting stale diff artifacts: always regenerate before triage.

## Recommended Regeneration Command

From `rust/di-compiler`:

```bash
cargo run -p fast-di-compile -- \
  --magento-root /var/www/application \
  --output /var/www/application/generated \
  --jobs 8 \
  --compare-archive \
  --archive-root /var/www/application/generated \
  --compare-report-dir /var/www/application/generated/diff
```

Then run the parity script again.

## Minimal Debug Loop

1. Rebuild + generate + compare.
2. Run parity script for `--area=global`.
3. Fix one root-cause bucket.
4. Re-run script.
5. Repeat until global is clean, then handle area overlays.

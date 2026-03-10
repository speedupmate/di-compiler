# fast-di-compile (Rust Magento DI Compiler)

Rust workspace that replaces `bin/magento setup:di:compile` with a faster pipeline while tracking Magento parity against archive truth:

- code truth: `generated/_code/`
- metadata truth: `generated/_metadata/`

## Goals

- Generate Magento DI code (`generated/code`) and metadata (`generated/metadata`) with parity-focused behavior.
- Provide deterministic archive comparison outputs for regression detection.
- Keep iteration fast via parallel parsing, incremental writes, and persistent PHP reflection workers.

## Workspace Layout

- `crates/cli` (`fast-di-compile`): orchestration, phase pipeline, archive compare, reporting.
- `crates/php-extractor`: PHP class scanning + extraction (lexer/tree-sitter/PHP fallback).
- `crates/di-xml-reader`: DI XML discovery, parsing, and merge behavior.
- `crates/di-resolver`: interceptors/factories/proxies detection and argument resolution.
- `crates/code-generator`: PHP code + metadata serialization.
- `crates/validator`: output-vs-truth validation helpers.
- `.plans/`: project planning, feature specs, and tickets.

## Prerequisites

- Rust toolchain (edition 2021)
- PHP CLI available (`php` or custom path via `--fallback-php`)
- Magento source checkout (for `--magento-root`)

## Build and Test

From `rust/di-compiler`:

```bash
cargo fmt --all
cargo test --workspace
cargo check --workspace
```

## Compile Workflows

### 0) Bootstrap archive baseline (`_code`, `_metadata`) if missing

If `generated/_code` or `generated/_metadata` does not exist, create the baseline once from Magento:

```bash
cd /var/www/application
bin/magento setup:di:compile
mv generated/code generated/_code
mv generated/metadata generated/_metadata
```

After this bootstrap, Rust compare mode uses these folders as source of truth.

### 1) Generate to Magento `generated/`

```bash
cargo run -p fast-di-compile -- \
  --magento-root /var/www/application \
  --output /var/www/application/generated \
  --jobs 8
```

### 2) Generate + compare against archive baseline

```bash
cargo run -p fast-di-compile -- \
  --magento-root /var/www/application \
  --output /var/www/application/generated \
  --jobs 8 \
  --compare-archive \
  --archive-root /var/www/application/generated \
  --compare-report-dir /var/www/application/generated/diff
```

### 3) Fail CI if archive diff is not clean

```bash
cargo run -p fast-di-compile -- \
  --magento-root /var/www/application \
  --output /var/www/application/generated \
  --compare-archive \
  --archive-root /var/www/application/generated \
  --compare-report-dir /var/www/application/generated/diff \
  --compare-fail-on-diff
```

### 4) Validate against PHP-generated directory

```bash
cargo run -p fast-di-compile -- \
  --magento-root /var/www/application \
  --output /tmp/rust-di-out \
  --validate \
  --php-generated /var/www/application/generated
```

## Output Layout

- Code: `generated/code/**/*.php`
- Metadata: `generated/metadata/*.php`
- Archive compare reports (`--compare-archive`):
  - `generated/diff/summary.json`
  - `generated/diff/code.missing.txt`
  - `generated/diff/code.extra.txt`
  - `generated/diff/code.changed.txt`
  - `generated/diff/metadata.missing.txt`
  - `generated/diff/metadata.extra.txt`
  - `generated/diff/metadata.changed.txt`

### Comparable Metadata Reports

When archive compare runs, normalized metadata artifacts are also generated:

- `generated/diff/comparable_metadata/comparable_<file>.archive.json`
- `generated/diff/comparable_metadata/comparable_<file>.output.json`
- `generated/diff/comparable_metadata/comparable_<file>_report.json`
- `generated/diff/comparable_metadata/comparable_<file>_report.txt`
- `generated/diff/comparable_metadata/manifest.txt`

Use `*_report.txt` first for triage (severity, top mismatch sections/type pairs, fix-category hints), then inspect the paired JSON files for exact content drift.

## Parity Model

Magento DI behavior is split across:

- compile-time generation/scanning behavior (`setup:di:compile` analog)
- runtime autoload-triggered generation behavior (Factory/Proxy/Interceptor conventions)

This project targets compile pipeline parity and uses `generated/_code` + `generated/_metadata` as the baseline oracle for convergence.

## Recommended Debug Loop

1. Run compile with `--compare-archive`.
2. Open `generated/diff/summary.json`.
3. For metadata drift, inspect `generated/diff/comparable_metadata/*_report.txt`.
4. Patch resolver/generator logic.
5. Re-run and confirm counts drop.

## Planning and Ticket Workflow

- Planning index: `.plans/README.md`
- Ticket index: `.plans/.tickets/README.md`
- One execution slice per ticket; keep ticket status current when landing work.

## Performance

Typical clean run on a 12-CPU dev box with ~18,800 PHP files and 570 di.xml files:

| Phase | Time |
|-------|------|
| Phase 1+2 — PHP scan | ~830ms |
| Phase 3a/3b — di.xml parse | ~190ms |
| Phase 4–6 — detection + codegen | ~440ms |
| Phase 7 — metadata generation | ~1.0s |
| **Total** | **~2.4s** |

Use `--verbose` to see per-phase and Phase 7 sub-step timing.

## Known Limitations

- `code_extra=3`: three generated files differ from the PHP archive baseline (edge ordering cases).
- `metadata_changed=16`: key-ordering noise only; Magento runtime uses key lookup so there is no behavioral impact. Exact byte-for-byte parity is a known deferred item (TKT-034).
- Composer-seeded module root discovery not yet implemented; falls back to `registration.php` DFS (TKT-035).

## License

MIT License. See [LICENSE](LICENSE).

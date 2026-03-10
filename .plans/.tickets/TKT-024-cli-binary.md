---
id: TKT-024
title: CLI binary
phase: 06-cli
feature: cli-binary
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-001, TKT-008, TKT-011, TKT-015, TKT-016, TKT-017, TKT-018, TKT-019, TKT-020, TKT-021, TKT-023]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - fast-di-compile --magento-root /var/www/application produces correct generated/
  - --dry-run writes nothing
  - --validate exits 1 on diff
  - Progress bar visible during run
  - --verbose prints extraction stats and per-phase timing
---

# TKT-024: CLI Binary

## Scope

Wire all crates into the `fast-di-compile` binary with clap + indicatif.

## Implementation Notes

```rust
#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = ".")]
    magento_root: PathBuf,

    #[arg(long)]
    output_dir: Option<PathBuf>,

    #[arg(long, default_value_t = num_cpus::get())]
    jobs: usize,

    #[arg(long)]
    area: Option<String>,

    #[arg(long, default_value_t = true)]
    fallback_php: bool,

    #[arg(long)]
    validate: bool,

    #[arg(long)]
    verbose: bool,

    #[arg(long)]
    dry_run: bool,

    #[arg(long)]
    incremental: bool,
}
```

Orchestration in `main()`:
1. Walk PHP files (TKT-002)
2. Extract ClassInfo (TKT-008) with rayon
3. Parse + merge di.xml (TKT-010, 011, 012)
4. Resolve (TKT-013, 014, 015)
5. Generate + write (TKT-016–022) unless `--dry-run`
6. If `--validate`: run validator (TKT-023), exit 1 if not clean

## Exit Codes

- 0: success
- 1: validation diff found
- 2: extraction failure (PhpFallbackFailed)
- 3: invalid arguments

# Phase 06: CLI

## Purpose

Build the `cli` crate. Wire all crates into the `fast-di-compile` binary with full
`clap`-based argument parsing and `indicatif` progress reporting.

## Gate To Enter

Phases 01–05 complete (all crates functional, validator green).

## Gate To Complete

- Binary runs end-to-end with `--magento-root` and produces correct `generated/` output
- All CLI flags functional
- `--validate` flag runs diff harness and exits non-zero on any diff
- `--dry-run` runs extract+resolve but writes nothing

## Features In This Phase

| Feature | Deps |
|---------|------|
| [21-cli-binary](../.features/21-cli-binary.md) | all crates |

## CLI Interface

```
fast-di-compile [OPTIONS]

Options:
  --magento-root <PATH>   Default: current directory
  --output-dir <PATH>     Default: <root>/generated
  --jobs <N>              Parallel workers. Default: num_cpus
  --area <AREA>           Compile specific area only. Default: all
  --fallback-php          Enable PHP fallback for unparseable files [default: on]
  --validate              After compile, diff against PHP output
  --verbose               Show per-file timing
  --dry-run               Extract + resolve but don't write files
  --incremental           Skip files where source hash unchanged
```

## Tickets In This Phase

TKT-024

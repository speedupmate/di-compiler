# 21: CLI Binary

- Category: Infrastructure
- Status: Planned
- Implementation Phase: 06-cli
- Owner: Unassigned
- Feature ID: `cli-binary`
- Suggested Dependencies: All crates

## Intent

Wire all crates into the `fast-di-compile` binary with `clap` argument parsing,
`indicatif` progress bars, and `env_logger` logging.

## CLI Interface

```
fast-di-compile [OPTIONS]

Options:
  --magento-root <PATH>   Magento installation root [default: .]
  --output-dir <PATH>     Output directory [default: <root>/generated]
  --jobs <N>              Parallel workers [default: num_cpus]
  --area <AREA>           Compile specific area only [default: all]
  --fallback-php          Enable PHP fallback for unparseable files [default: true]
  --no-fallback-php       Disable PHP fallback
  --validate              After compile, diff against PHP output and exit non-zero if mismatch
  --verbose               Show per-file timing and extraction stats
  --dry-run               Extract + resolve but don't write files
  --incremental           Skip files where source hash unchanged
```

## Core State and Actions

```rust
// src/main.rs in cli crate
fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    env_logger::init();
    let config = CompileConfig::from(&args);

    let pb = ProgressBar::new_spinner();

    // 1. Discover PHP files
    // 2. Extract ClassInfo (parallel)
    // 3. Parse + merge di.xml
    // 4. Resolve DI graph
    // 5. Generate code files (parallel)
    // 6. Write metadata files
    // 7. Optionally validate

    pb.finish_with_message("Done");
    Ok(())
}
```

## Acceptance Criteria

- `--dry-run` writes nothing to disk
- `--validate` exits 1 on any diff
- Progress bar shows file processing progress
- `--verbose` prints per-phase timing and extraction stats summary
- Exit code 0 = success, 1 = diff found, 2 = extraction failure

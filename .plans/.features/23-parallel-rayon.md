# 23: Parallel File Parsing (rayon)

- Category: Performance
- Status: Planned
- Implementation Phase: 07-performance
- Owner: Unassigned
- Feature ID: `parallel-rayon`
- Suggested Dependencies: 06-extract-result-type, 19-metadata-php-serializer

## Intent

Replace sequential file processing loops with `rayon::par_iter()` in the PHP extraction
and code generation phases. Measure before/after wall clock. Must not change output.

## Parallelizable Phases

| Phase | Safe to parallelize? |
|-------|---------------------|
| PHP file extraction | Yes — files are independent |
| di.xml parsing | Yes — files are independent |
| di.xml merging | No — must be sequential in load order |
| DI resolution | No — graph traversal (sequential) |
| Code generation (write files) | Yes — output files are independent |
| Metadata file writing | No — one file per area (few files) |

## Core State and Actions

```rust
// php-extractor
let results: Vec<ExtractResult> = file_paths
    .par_iter()
    .map(|p| extract_file(p, &config))
    .collect();

// code-generator
interceptor_specs.par_iter().for_each(|spec| {
    let content = generate_interceptor(spec, &class_map[&spec.fqcn]);
    write_if_changed(&output_path(spec), &content);
});
```

## Acceptance Criteria

- Output unchanged after parallelization (TKT-023 still green)
- Wall clock measurably reduced (benchmark before/after)
- No data races (Rust borrow checker enforces this)
- Thread pool size controlled by `--jobs` CLI flag

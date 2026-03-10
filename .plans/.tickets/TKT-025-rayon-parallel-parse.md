---
id: TKT-025
title: rayon parallel file parsing + code generation
phase: 07-performance
feature: parallel-rayon
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-008, TKT-022]
touches:
  - rust/di-compiler/crates/php-extractor/src/lib.rs
  - rust/di-compiler/crates/code-generator/src/lib.rs
acceptance:
  - TKT-023 still green after parallelization
  - Measurable wall clock reduction (log before/after)
  - --jobs N controls rayon thread pool size
---

# TKT-025: rayon Parallel File Parsing + Code Generation

## Scope

Apply `rayon::par_iter()` to the two most parallelizable phases: PHP file extraction and code file writing.

## Implementation Notes

**PHP extraction** (in `extract_all`):
```rust
let results: Vec<ExtractResult> = file_paths
    .par_iter()
    .map(|p| extract_file(p, &config))
    .collect();
```

**di.xml parsing** (files are independent, merge is sequential):
```rust
let partial_configs: Vec<_> = di_xml_paths
    .par_iter()
    .map(|p| parse_di_xml(p))
    .collect::<Result<Vec<_>, _>>()?;
// then merge sequentially in load order
```

**Code generation** (output files are independent):
```rust
interceptor_specs.par_iter().for_each(|spec| {
    let content = generate_interceptor(spec, &class_map[&spec.fqcn]);
    write_if_changed(&output_path_for(spec), &content).unwrap();
});
```

Control thread pool:
```rust
rayon::ThreadPoolBuilder::new()
    .num_threads(config.jobs)
    .build_global()?;
```

## Risks

- `write_if_changed` must be thread-safe (it is, using separate paths per file)
- di.xml merge must NOT be parallelized (order-dependent)

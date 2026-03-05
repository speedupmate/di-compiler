---
id: TKT-022
title: Content-addressed file writes
phase: 04-code-generator
feature: (cross-cutting)
owner: Unassigned
status: Ready
estimate: S
depends_on: [TKT-017, TKT-018, TKT-019, TKT-020, TKT-021]
touches:
  - rust/di-compiler/crates/code-generator/src/writer.rs
acceptance:
  - Files with unchanged content are not written to disk
  - New and changed files are written correctly
  - Reduces FS writes on incremental runs
---

# TKT-022: Content-Addressed File Writes

## Scope

Wrap all file writes with a hash-before-write check to skip unchanged files.

## Implementation Notes

```rust
pub fn write_if_changed(path: &Path, content: &str) -> io::Result<WriteResult> {
    if path.exists() {
        let existing = std::fs::read_to_string(path)?;
        if existing == content {
            return Ok(WriteResult::Skipped);
        }
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, content)?;
    Ok(WriteResult::Written)
}
```

For better performance use `rustc-hash` to compare hashes instead of full string equality.

## Risks

- Must not use this as a substitute for TKT-026 (incremental fingerprinting) — this is just an I/O optimization

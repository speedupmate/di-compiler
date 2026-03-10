---
id: TKT-042
title: Persistent PHP worker for reflection hot paths
phase: 09-performance-hardening
feature: php-reflection-worker-pool
owner: Unassigned
status: Done
estimate: M
depends_on: [TKT-035]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
acceptance:
  - Reflection-heavy phases no longer spawn one PHP process per reflected class
  - Phase timings show major drop in reflection-bound phases while parity counts remain stable
  - Worker lifecycle is bounded (startup, request/response, cleanup)
test_plan:
  - Run `cargo test -p fast-di-compile --no-run`
  - Run full compile with `--compare-archive` and capture phase timings before/after
  - Verify compare summary counts remain unchanged
---

# TKT-042: Persistent PHP worker for reflection hot paths

## Scope

Replace per-call `php -r` reflection execution with a persistent worker request model to remove repeated autoload overhead.

## Implementation Notes

- Use a line-oriented protocol with one JSON response per request.
- Support method and constructor reflection request types.
- Retry once when a worker exits unexpectedly.
- Keep reflected signature normalization logic unchanged.

## Current Snapshot (2026-03-06)

- Implementation is active in local working tree (`crates/cli/src/main.rs`) and benchmarked.
- Warm run example:
  - Total: `5.029s`
  - Phase 4: `0.659s`
  - Phase 6: `0.150s`
- Archive compare counts unchanged in the same run:
  - code missing `0`, extra `39`, changed `32`
  - metadata missing `0`, extra `0`, changed `16`

## Risks

- Worker protocol failures can silently degrade reflection coverage if not surfaced clearly.
- Persistent workers increase lifecycle complexity (cleanup, crash recovery).

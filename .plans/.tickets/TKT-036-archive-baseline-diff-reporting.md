---
id: TKT-036
title: Archive baseline diff reporting in fast-di-compile
phase: 09-performance-hardening
feature: archive-baseline-diff-reporting
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-034]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/.plans/.tickets/README.md
acceptance:
  - `fast-di-compile` supports an archive comparison mode against `_code` and `_metadata`
  - Comparison writes stable report files for missing/extra code and metadata paths
  - CLI logs summary counts after build so parity status is visible without manual shell diff commands
  - Optional non-zero exit on diff is available for CI gating
test_plan:
  - Run compile with `--compare-archive` and verify `summary.json` plus four list files are written
  - Confirm summary counts match manual `comm`-style file set difference on the same output
  - Run with `--compare-fail-on-diff` and verify exit code is `1` when differences exist
---

# TKT-036: Archive baseline diff reporting in fast-di-compile

## Scope

Add a first-class archive comparison step to `fast-di-compile` so every build can emit parity status against Magento archive baselines (`generated/_code`, `generated/_metadata`) without ad-hoc shell commands.

## Implementation Notes

- Keep the compare step optional behind explicit CLI flags.
- Keep report format deterministic and machine-friendly (`*.txt` + `summary.json`).
- Use relative paths in report files so results remain portable between environments.

## Implementation Update (2026-03-06)

- Landed via commit `d50699a`.
- `--compare-archive`, `--archive-root`, `--compare-report-dir`, and optional fail-on-diff gating are now first-class CLI flow.

## Risks

- Comparing against an incorrect archive root can produce noisy or misleading diffs.
- Missing archive folders must fail fast with a clear error message.

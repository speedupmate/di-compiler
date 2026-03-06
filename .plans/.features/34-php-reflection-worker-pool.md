# 34: PHP Reflection Worker Pool

- Category: Performance
- Status: In Progress
- Implementation Phase: 09-performance-hardening
- Owner: Unassigned
- Feature ID: `php-reflection-worker-pool`
- Suggested Dependencies: 21-cli-binary, 23-parallel-rayon

## Intent

Eliminate repeated PHP process boot + autoload costs from reflection-heavy paths by introducing a persistent PHP worker model and request batching behavior in the CLI runtime.

## User Behavior

1. `fast-di-compile` completes reflection-heavy phases (Phase 4 and Phase 6) without spawning one `php -r` subprocess per class.
2. Warm runs remain near the 5-second target while preserving existing archive diff counts.

## Core State and Actions

1. Start a bounded pool of persistent PHP workers once per run.
2. Send line-oriented reflection requests (`methods:<FQCN>`, `ctor:<FQCN>`).
3. Retry once with a fresh worker if the checked-out worker dies.
4. Keep output parsing and normalization behavior unchanged.

## Runtime Effects

1. Significant reduction in wall-clock time for reflection steps.
2. Reduced process churn and autoload overhead.

## Implementation Steps

1. Replace ad-hoc `php -r` reflection calls with worker-pool-backed request paths.
2. Keep phase timers enabled and compare pre/post timings.
3. Validate parity using archive compare reports.

## Test Plan

1. Run full compile with `--compare-archive` and record phase timings.
2. Verify compare counts are unchanged versus pre-worker branch.
3. Run `cargo test -p fast-di-compile --no-run`.

## Acceptance Criteria

- Warm run total is near 5 seconds on this install.
- Phase 4 and Phase 6 no longer dominate runtime due to PHP boot overhead.
- No regression in archive compare counts.

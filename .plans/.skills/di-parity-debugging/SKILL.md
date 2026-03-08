---
name: di-parity-debugging
description: Use this skill when debugging fast-di-compile parity issues (metadata/code drift, missing Magento CLI commands, runtime compiled-mode failures) against generated/_code and generated/_metadata baselines.
---

# DI Parity Debugging

Use this workflow for fast, repeatable diagnosis of Magento DI parity regressions.

## Baseline

- Source of truth:
  - `generated/_code`
  - `generated/_metadata`
- Rust output:
  - `generated/code`
  - `generated/metadata`
- Diff reports:
  - `generated/diff/*`
  - `generated/diff/comparable_metadata/*`

If `_code` / `_metadata` are missing, bootstrap them once from Magento:

```bash
cd /var/www/application
bin/magento setup:di:compile
mv generated/code generated/_code
mv generated/metadata generated/_metadata
```

## Core Loop

1. Run compiler with archive compare enabled.
2. Read `generated/diff/summary.json`.
3. Triage metadata with `comparable_metadata/*_report.txt` first.
4. Drill into exact normalized deltas via paired:
   - `comparable_<file>.archive.json`
   - `comparable_<file>.output.json`
5. Classify root cause:
   - argument merge/resolution parity
   - preferences/instanceTypes mapping parity
   - plugin-list/interception key-space parity
   - scalar/container type normalization parity
6. Patch Rust resolver/generator logic.
7. Re-run compare and track count reductions.

## Runtime Safety Check

After parity changes, verify compiled-mode runtime behavior:

1. regenerate metadata/code
2. run `bin/magento` (or `bin/magento list`)
3. if storefront/admin issues appear, map stack traces to metadata sections:
   - `Factory/Compiled` + class not found -> `instanceTypes` / virtual-type resolution
   - missing CLI commands -> interface arg inheritance + recursive argument merge
   - config source warnings/errors -> malformed `arguments.*` structure (shape/type drift)

## Reference Notes

Load these only when needed:

- `../../.debugging/metadata_diff_and_reflection_oracle.md`
  - comparable-diff pattern, structured mismatch debugging
- `../../.debugging/interface_arg_inheritance.md`
  - missing command root cause and merge-order fixes
- `../../.debugging/debugging_draft.md`
  - compiled-mode virtual type chain failure analysis
- `../../.debugging/firebear_dependency_injection_article_summary.md`
  - conceptual background; not source-of-truth for 2.4.8 behavior

## Decision Rules

- Do not trust raw text diffs for metadata parity; use normalized comparable reports.
- Prioritize high-risk type mismatches (`NULL|object`, `object|array`, `NULL|array`) before value mismatches.
- Treat Magento core behavior as oracle when docs/blogs conflict.
- Keep fixes semantic: no temporary shape hacks that only silence one stack trace.

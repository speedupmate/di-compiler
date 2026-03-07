# 33: Code Content Parity Closure

- Category: Correctness
- Status: In Progress
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `code-content-parity-closure`
- Suggested Dependencies: 26-interceptor-namespace-structure-parity, 27-interceptor-method-signature-parity, 29-extension-attributes-generation, 32-runtime-map-generator-coverage

## Intent

Close remaining content-level diffs between `generated/code` and `generated/_code` after path-level parity, with a root-cause-driven sequence that prioritizes high-count drift patterns first.

## Current Gap

- Path parity is near closure, but content parity still shows multi-pattern drift:
  - Metadata key-space gaps (arguments/interception/plugin-list) tied to incomplete type-universe coverage
  - Plugin-list processed-key inflation (`_execute___self` family) and scope leakage
  - Proxy ordering/surface/class-shape mismatches
  - Extension artifact ordering drift
  - Factory edge cases (`*ExtensionInterfaceFactory`, global namespace factory)

Current archive compare snapshot (2026-03-06):

- code missing `0`, extra `39`, changed `32`
- metadata missing `0`, extra `0`, changed `16`
- Completed slices: TKT-038, TKT-039
- Open slices: TKT-040, TKT-041, TKT-043, TKT-044, TKT-045, TKT-046

## Implementation Steps

1. Establish metadata type-universe contract (TKT-043), then apply it to:
   - area/global argument resolution for virtual/generated keys (TKT-044)
   - interception registry completeness for virtual/generated keys (TKT-044)
   - plugin-list key-space/scope parity (TKT-045)
2. Fix proxy content parity slice (TKT-046):
   - method ordering and method-surface parity
   - class-shape (`extends`/`implements`) and return-type fidelity
3. Fix extension/factory content parity slice (TKT-040):
   - align extension attribute method ordering with baseline behavior
   - handle `*ExtensionInterfaceFactory` target mapping and global-namespace factory output
4. Run TKT-041 final convergence pass with categorized residuals and documentation.

## Test Plan

1. Run full compile with `--compare-archive` and verify `code.changed.txt` trend by category.
2. Add focused unit tests for each fixed pattern (interceptor/proxy/factory/extension).
3. Validate no regressions in `_metadata` generation and no reintroduction of path-level misses.

## Acceptance Criteria

- Interceptor content diffs no longer dominated by inherited-surface method over-generation.
- Proxy diffs for default literal rendering and interface/class declaration trend to zero.
- Extension/factory known pattern buckets are closed or reduced to explicitly documented residuals.
- Residual changed files are small, categorized, and traceable to specific documented constraints.

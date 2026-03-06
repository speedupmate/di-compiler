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
  - Interceptor over-generation and constructor signature loss
  - Proxy default-value and interface/class fallback mismatches
  - Extension artifact ordering drift
  - Factory edge cases (`*ExtensionInterfaceFactory`, global namespace factory)

## Implementation Steps

1. Fix interceptor content parity slice:
   - resolve plugin method sets more reliably
   - avoid fallback behavior that emits full inherited method surfaces
   - preserve inherited constructor signatures for interceptor generation
2. Fix proxy content parity slice:
   - preserve literal default values from signatures
   - classify proxy targets as interface/class using composer fallback when not in scanned class map
3. Fix extension/factory content parity slice:
   - align extension attribute method ordering with baseline behavior
   - handle `*ExtensionInterfaceFactory` target mapping and global-namespace factory output
4. Re-run archive compare with changed-file reporting and iterate until content-diff trend reaches low residual set.

## Test Plan

1. Run full compile with `--compare-archive` and verify `code.changed.txt` trend by category.
2. Add focused unit tests for each fixed pattern (interceptor/proxy/factory/extension).
3. Validate no regressions in `_metadata` generation and no reintroduction of path-level misses.

## Acceptance Criteria

- Interceptor content diffs no longer dominated by inherited-surface method over-generation.
- Proxy diffs for default literal rendering and interface/class declaration trend to zero.
- Extension/factory known pattern buckets are closed or reduced to explicitly documented residuals.
- Residual changed files are small, categorized, and traceable to specific documented constraints.

# 36: DI Merge Order and Null Argument Surface

- Category: Correctness
- Status: Done
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Estimate: Delivered
- Execution Status: Done
- Feature ID: `di-merge-order-and-null-surface`
- Suggested Dependencies: 13-arguments-resolver, 09-di-config-model, 35-hybrid-module-root-resolver

## Intent

Close the merge-order and null-surface correctness gaps in area-config metadata generation:

1. **Module-order preference resolution** — di.xml merge must sort by `(priority, config.php_position, path)` using the actual PHP module load sequence from `app/etc/config.php`.

2. **Null argument surface** — PHP emits `'ClassName' => NULL` in area config arguments for concrete classes with no resolved constructor arguments, including di.xml-only configured types.

## Current Status (2026-03-10)

- Achieved:
  - `missing=0` across all areas
  - `mismatches=0` across all areas
  - `preferences` and `instanceTypes` are zero-drift in all areas
- Residual:
  - `extra=40` is explicitly accepted for now and tracked as deferred follow-up in Feature 37 / TKT-055

## User Behavior

1. Running `fast-di-compile` with Hyva modules installed resolves area arguments with correct module-order precedence.
2. Area config files include expected NULL entries for concrete classes in the DI universe.

## Core State and Actions

1. `parse_config_php` preserves module load ordering used for DI merge tie-break.
2. Module index tie-break now uses deterministic config.php order.
3. `resolve_all_arguments_for_named_types` handles NULL emission for di.xml-only configured concrete types.

## Rendering Contract

1. Area config `arguments` section now matches PHP for missing/mismatch parity.
2. Preferences section reflects expected module precedence behavior.

## Implementation Steps

1. Implement deterministic module-order indexing from config.php.
2. Apply module-order index in DI merge tie-break sorting.
3. Expand NULL emission handling for di.xml-only configured concrete types.
4. Normalize setup runtime overrides used by metadata generation.

## Test Plan

1. Run `compare_metadata_parity.php` and verify all areas report `missing=0`, `mismatches=0`.
2. Validate setup/runtime override paths do not regress parity-critical arguments.
3. Keep archive compare and parity script outputs attached to phase notes.

## Acceptance Criteria

- All areas sustain `missing=0` and `mismatches=0` for area metadata parity.
- Residual `extra=40` is deferred and tracked separately (Feature 37 / TKT-055).
- Area metadata generation remains stable after subsequent parity fixes.

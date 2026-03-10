# 36: DI Merge Order and Null Argument Surface

- Category: Correctness
- Status: Planned
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Estimate: TBD
- Execution Status: Not Started
- Feature ID: `di-merge-order-and-null-surface`
- Suggested Dependencies: 13-arguments-resolver, 09-di-config-model, 35-hybrid-module-root-resolver

## Intent

Close the two remaining correctness gaps in area-config metadata generation:

1. **Module-order preference resolution** — di.xml merge must sort by `(priority, config.php_position, path)` using the actual PHP module load sequence from `app/etc/config.php`. The current implementation assigns module indices via `HashMap::iter().enumerate()` which has arbitrary order, making the tie-break key meaningless.

2. **Null argument surface** — PHP emits `'ClassName' => NULL` in area config arguments for any concrete class that has no configured constructor arguments, including types that appear in di.xml `<type>` entries even when no PHP file was found on disk. Our code currently only emits NULL for types in `base_class_fqcns` (source PHP scan), silently dropping di.xml-only configured types.

## User Behavior

1. Running `fast-di-compile` with Hyva modules installed produces `global.php` where `configStructure`-related arguments resolve to `Hyva\Checkout\Model\Config\Structure\Interceptor` (not the Magento core version).
2. Area config files contain NULL entries for all concrete classes in the DI universe — both source-scanned and di.xml-only configured types.

## Core State and Actions

1. `parse_config_php` returns an ordered structure (`Vec<(String, i64)>`) that preserves PHP array insertion order.
2. `enabled_modules` HashMap is built by enumerating that ordered Vec, so each module gets a stable positional index matching its config.php position.
3. `resolve_all_arguments_for_named_types` emits NULL for types that: appear in `di_config.type_configs`, are absent from `class_map`, don't end in `Interface`, and are not generated artifacts (`Interceptor`/`Factory`/`Proxy` suffixes).

## Rendering Contract

1. Area config `arguments` section: every concrete class in the type universe (source-scanned + di.xml-configured) appears, either with resolved args or as NULL.
2. Preferences section: Hyva overrides take effect because they have higher config.php position (377+) than Magento core (21).

## Implementation Steps

1. Change `parse_config_php` return type from `HashMap<String, i64>` to `Vec<(String, i64)>` — preserves PHP array insertion order.
2. Rebuild `enabled_modules` from `Vec::enumerate()` instead of `HashMap::iter().enumerate()`.
3. Pass `&enabled_modules` to `find_di_xml_files` variants (already wired; this fix makes the indices meaningful).
4. In `resolve_all_arguments_for_named_types`: add branch for types not in class_map — emit NULL if in `type_configs` and not interface/generated.

## Test Plan

1. Unit test: parse a PHP config array literal in two files with reversed module order; assert the Vec indices reflect file order, not hash order.
2. Integration: compile with Hyva modules; assert `configStructure` in `frontend.php` resolves to `Hyva\Checkout\...Interceptor`.
3. Assert `check_all_areas.php` missing count drops from 87–155 toward zero for the di.xml-only NULL cases.

## Acceptance Criteria

- `check_all_areas.php` mismatches drop from 173–189 to < 50 across all areas after TKT-048.
- No new `extra` entries introduced by TKT-052 NULL emission widening.
- All area config PHP files pass `php -n -l`.

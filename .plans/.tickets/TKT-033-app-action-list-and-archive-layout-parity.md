---
id: TKT-033
title: App action list and archive layout parity
phase: 08-parity-closure
feature: app-action-list-and-output-layout
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-032]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/code-generator/src/
  - rust/di-compiler/.plans/README.md
acceptance:
  - app_action_list.php is generated and matches archived metadata baseline
  - Validation workflow explicitly supports archived _code/_metadata baseline paths
  - Metadata missing count decreases by app_action_list artifact
test_plan:
  - Assert app_action_list.php presence and parseability
  - Compare action list mapping with archived baseline
  - Run validator in archive-baseline mode
---

# TKT-033: App action list and archive layout parity

## Scope

Implement missing app action list metadata output and remove ambiguity around compare layout.

## Implementation Notes

- Add generation path for `app_action_list.php`.
- Keep docs and CLI usage aligned to archived baseline comparison process.

## Risks

- Output layout assumptions can mask true parity changes if compare paths are inconsistent.

# 31: App Action List and Output Layout Parity

- Category: Correctness
- Status: In Progress
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `app-action-list-and-output-layout`
- Suggested Dependencies: 21-cli-binary, 30-plugin-list-metadata-generation

## Intent

Close remaining metadata/output layout mismatches around app action list generation and archive comparison workflow.

## Current Gap

- Missing `app_action_list.php` metadata output.
- Default Rust output layout uses `generated/code` and `generated/metadata` while repo baseline is archived under `generated/_code` and `generated/_metadata`.

## Implementation Steps

1. Implement `app_action_list.php` generation equivalent to Magento compile operation.
2. Add explicit comparison mode/config docs for archived `_code` / `_metadata` baseline.
3. Ensure validation workflow compares the correct source-of-truth paths.

## Test Plan

1. Confirm `app_action_list.php` exists and parses.
2. Compare action list keys against archived baseline.
3. Run end-to-end validator in archive-baseline mode.

## Acceptance Criteria

- `app_action_list.php` emitted with matching content
- Validation docs and CLI usage clearly target archive baseline paths

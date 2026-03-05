# 25: Metadata Validity Parity

- Category: Correctness
- Status: Planned
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Feature ID: `metadata-validity-parity`
- Suggested Dependencies: 19-metadata-php-serializer, 20-validator-harness

## Intent

Ensure all Rust-generated metadata PHP files are syntactically valid and serialized with Magento-compatible scalar formatting.

## Current Gap

- Area metadata files fail `php -n -l` due invalid numeric literal output for leading-zero numeric strings.

## Implementation Steps

1. Remove unsafe numeric auto-coercion for scalar strings in metadata serializers.
2. Add metadata syntax lint checks to validator/integration workflow.
3. Keep serializer output deterministic and compatible with Magento var_export output.

## Test Plan

1. Lint every `generated/metadata/*.php` file with `php -n -l`.
2. Validate no syntax errors in all areas.
3. Re-run diff harness against archived metadata baseline.

## Acceptance Criteria

- All generated metadata files pass `php -n -l`
- No invalid numeric literal output
- Metadata diff count decreases from current baseline

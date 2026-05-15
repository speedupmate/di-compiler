# 39: Magento Constructor Integrity Compatibility

- Category: Correctness
- Status: Implemented
- Implementation Phase: 08-parity-closure
- Owner: Unassigned
- Estimate: Delivered
- Execution Status: Done
- Feature ID: `magento-constructor-integrity-compatibility`
- Suggested Dependencies: 13-arguments-resolver, 21-cli-binary

## Intent

Keep `fast-di-compile` aligned with Magento's own constructor integrity limitations instead of accepting PHP constructor signatures that Magento cannot compile.

Magento's constructor validator treats only the scalar/pseudo type hints returned by `Magento\Framework\Code\Reader\ScalarTypesProvider::getTypes()` as non-class types:

- `array`
- `string`
- `int`
- `integer`
- `float`
- `bool`
- `boolean`
- `mixed`
- `callable`

Unsupported bare pseudo-types such as `object` and `iterable` are namespace-resolved by Magento as class names. A parent constructor using `object $arg`, followed by a child forwarding a real interface to `parent::__construct($arg)`, fails Magento's compile-time constructor integrity validation. Rust must fail the same case by default.

## User Behavior

1. Running `fast-di-compile` on code Magento cannot compile due to unsupported constructor pseudo-types fails loudly.
2. The error message includes the violating child file, required type, actual type, and Magento-supported scalar/pseudo type allowlist.
3. Users can pass `--ignore-constructor-integrity` to continue for debugging or migration work, but this is explicitly non-default.

## Core State and Actions

1. Constructor argument resolution mirrors Magento's namespace-resolution behavior for unsupported pseudo-types.
2. `object` and `iterable` are not treated as safe primitive-like constructor types.
3. `mixed` remains supported because Magento's `ScalarTypesProvider` allows it.
4. CLI validation runs after class extraction and before DI metadata/code generation.

## Runtime Effects

1. Default compile fails before generation when constructor integrity violations are found.
2. `--ignore-constructor-integrity` logs violations and continues.

## Implementation Notes

Implemented in commit `9a9aeb0` (`Align DI metadata parity with Magento compiler`).

Key files:

- `crates/cli/src/main.rs`
- `crates/di-resolver/src/arguments.rs`

## Test Plan

1. Unit tests verify `object` and `iterable` are namespace-resolved like Magento.
2. Unit test verifies `mixed` remains a supported scalar/pseudo type.
3. Unit tests verify constructor integrity detects an `object` parent type mismatch and allows `mixed`.
4. Probe module dry-run fails by default and succeeds with `--ignore-constructor-integrity`.
5. `cargo test -p fast-di-compile -p di-resolver -p di-xml-reader -p code-generator` passes.

## Acceptance Criteria

- Rust compiler fails by default for constructor signatures Magento would reject.
- Error output lists Magento-supported scalar/pseudo constructor type hints.
- Ignore flag exists and is explicit.
- No broad false positives from normal parent/child constructor compatibility.

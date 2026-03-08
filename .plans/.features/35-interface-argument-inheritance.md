# 35: Interface Argument Inheritance

- Category: Correctness
- Status: Done
- Implementation Phase: 03-di-resolver
- Owner: Unassigned
- Feature ID: `interface-argument-inheritance`
- Suggested Dependencies: 13-arguments-resolver, 09-di-config-model

## Intent

Ensure that DI arguments registered against an interface type flow into the preference
concrete that implements the interface, replicating the behavior of PHP's
`Config::_collectConfiguration` + `ClassReader::getParents`.

PHP's `ClassReader::getParents()` returns both the parent class chain **and** all
directly-implemented interfaces for a given class. When `Config` builds argument
configurations it merges all entries in this full chain, so an interface's `<arguments>`
block in `di.xml` applies to every concrete that prefers that interface.

Without this, arguments registered on interfaces (e.g., `CommandListInterface` holding
`commands` entries from 50+ modules) were invisible when resolving the preference concrete
(`CommandList`), producing a truncated and incorrect argument set.

## Core Behavior

The fix lives in `merged_di_arguments_for_type_name` in
`crates/di-resolver/src/arguments.rs` and consists of two parts:

**Part 1 — Interface injection into the merge order**

For each class in the `extends` chain, compute the interfaces that are "new" at that
level: `new_interfaces = class.implements - parent.implements` (mirrors PHP
`array_diff(class.implements, parent.implements)`). Insert those interfaces into the merge
list just before the class itself so they contribute at lower priority than the class's
own args but higher priority than any ancestor.

**Part 2 — Recursive array merge instead of replacement**

When a same-name argument already exists in the accumulated result, array-type arguments
must be merged recursively by key (mirrors PHP `array_replace_recursive`) rather than
replaced wholesale. This required changing the return type of
`merged_di_arguments_for_type_name` from `Vec<&'a Argument>` (borrowed) to
`Vec<Argument>` (owned) and introducing a `merge_argument_into()` helper.

Without Part 2, an interface contributing 103 `commands` items would be overwritten by
the concrete's own 3-item `commands` override, discarding 100 entries.

## PHP Reference

- `Magento\Setup\Module\Di\Compiler\Config::_collectConfiguration`
- `Magento\Framework\Code\Reader\ClassReader::getParents`
- PHP built-in `array_replace_recursive` (the merge semantics this replicates)

## Acceptance Criteria

- `bin/magento list` reports the same command count in compiled mode as in developer mode (177 = 177)
- Arguments for `Magento\Framework\Console\CommandList` verified against PHP runtime `$config->getArguments()` — all 103 `commands` items present
- No regression in other classes: spot-checked multiple preference concretes against PHP runtime, all match

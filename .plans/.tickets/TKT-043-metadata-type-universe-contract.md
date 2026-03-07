---
id: TKT-043
title: Metadata type-universe contract (real + virtual + generated)
phase: 08-parity-closure
feature: runtime-map-generator-coverage
owner: Unassigned
status: Ready
estimate: M
depends_on: [TKT-034]
touches:
  - rust/di-compiler/crates/cli/src/main.rs
  - rust/di-compiler/crates/di-resolver/src/arguments.rs
  - rust/di-compiler/crates/code-generator/src/plugin_list.rs
  - rust/di-compiler/.plans/.tickets/
acceptance:
  - One explicit, deterministic "metadata type universe" is defined and used by metadata generation paths
  - Universe includes class-map types, di virtual types, and generated/runtime artifacts required by Magento metadata outputs
  - Downstream tickets can consume the universe without re-defining inclusion rules
test_plan:
  - Add unit tests for universe assembly and deterministic ordering
  - Add snapshot of per-scope type counts consumed by area config/interception/plugin-list paths
  - Validate no archive compare regressions on unchanged paths
---

# TKT-043: Metadata type-universe contract (real + virtual + generated)

## Context

Current metadata drift is spread across `global.php`, `interception.php`, and plugin-list files because each path uses a slightly different source set of classes. This ticket establishes the shared contract first so feature slices do not diverge.

## Scope

Define and wire a shared type-universe builder for metadata stages:

- real extracted classes (`class_map`)
- virtual types from merged DI config
- generated/runtime classes that must appear in compiled metadata (interceptors/factories/proxies/etc. as applicable)

This is a coordination ticket: it defines the seam and deterministic behavior that TKT-044/045 depend on.

## Risks

- Over-expanding the universe can inflate metadata with non-Magento entries.
- Under-expanding the universe leaves current missing buckets unresolved.

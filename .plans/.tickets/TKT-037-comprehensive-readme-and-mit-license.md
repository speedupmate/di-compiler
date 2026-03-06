---
id: TKT-037
title: Comprehensive README plan and MIT licensing
phase: 09-performance-hardening
feature: project-docs-and-licensing
owner: Unassigned
status: Done
estimate: S
depends_on: [TKT-036]
touches:
  - rust/di-compiler/README.md
  - rust/di-compiler/LICENSE
  - rust/di-compiler/Cargo.toml
  - rust/di-compiler/.plans/.tickets/README.md
acceptance:
  - A comprehensive root `README.md` exists and is aligned with the current CLI and architecture
  - README includes quickstart commands, validation/compare workflow, and baseline-diff report usage
  - README documents known parity state and contribution workflow (tickets, tests, commits)
  - MIT `LICENSE` exists at repo root and Cargo workspace metadata declares `license = "MIT"`
test_plan:
  - Verify README command examples run on this workspace (`cargo run -p fast-di-compile ...`)
  - Verify README references existing files/paths/flags only
  - Verify `LICENSE` is valid MIT text and present at repo root
  - Verify `cargo metadata` reflects workspace license field
---

# TKT-037: Comprehensive README plan and MIT licensing

## Scope

Define and implement a full project README and formal MIT licensing so new contributors can run, validate, and extend the compiler without tribal knowledge.

## README Structure Plan

1. Project intent and current parity status.
2. Repository layout and crate responsibilities.
3. Prerequisites and local setup.
4. Compile workflows:
   - standard run
   - archive compare (`--compare-archive`)
   - validation mode (`--validate --php-generated`)
5. Output layout:
   - `generated/code`
   - `generated/metadata`
   - diff reports (`summary.json`, `*.missing.txt`, `*.extra.txt`)
6. Magento parity model:
   - compile-time scanner behavior vs runtime generation behavior
   - baseline source-of-truth (`generated/_code`, `generated/_metadata`)
7. Testing and quality gates (`cargo fmt`, crate tests, parity checks).
8. Planning workflow (`.plans/.features`, `.plans/.tickets`) and ticket execution expectations.
9. Known limitations and next milestones.

## Risks

- README can drift if CLI flags or output format change without documentation updates.
- Ambiguous parity wording can cause contributors to “fix” expected scanner/runtime differences incorrectly.

## Implementation Update (2026-03-06)

- Landed via commit `1880873`.
- Root README and MIT licensing are in place and aligned with current CLI usage.

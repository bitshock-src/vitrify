---
sdd: [scope-slug]
title: [Title]
schema_version: 2.1
version: 0.1.0
status: Draft
tdd: ../../docs/tdd/[scope].md
tdd_version: [rev] (Approved, [YYYY-MM-DD])
created: [YYYY-MM-DD]
updated: [YYYY-MM-DD]
scope_roots:
  - [src/scope/]
  - [tests/scope/]
---

> Template for a bundle-shape SDD instance (`specs/<scope>/`). Structure only —
> all rules live in the SDD schema (`vitrify schema show`). Contracts live in
> `contracts/C-###-<slug>.md`, work units in `units/U-###-<slug>.md`, the
> derived ledger in `ledger.md` (never hand-edited; `vitrify ledger regen`),
> the append-only journal in `journal.ndjson`, change entries in
> `changelog.md`, decisions and open questions in `decisions.md`.

## Overview

[What is being built. Scope boundary: in and out. The pinned TDD named above
is the design authority.]

## Ground Rules

[Definitions, normative-language note, and the fixed conventions every
implementer obeys — including the test posture: which tests exist before the
build, which units write tests, and whether tests precede implementation
(sdd-schema §10). Part of every unit's reading set and of the `global`
trace hash.]

## Technology Decisions

[Language, runtime, every dependency with its exact pinned version. Part of
every unit's reading set and of the `global` trace hash.]

### Commands

- Bootstrap: [exact command]
- Build:     [exact command]
- Test:      [exact command, or none — no gate runs it]
- Lint:      [exact command]

## System Layout & Ownership

[Target layout table: every path under `scope_roots` owned by exactly one
unit, or listed here:]

unmanaged:
  - [path-no-unit-owns]

## Interface Contracts

[Index of `contracts/C-###-<slug>.md`. Every cross-unit surface is frozen in
exactly one contract before its consumers go Ready.]

## Work Units

[Index of `units/U-###-<slug>.md`.]

## Execution Order

[Foundation-first phases; `[P]` groups of mutually independent units.]

## Verification Gates

[Scope-specific gate thresholds and where build evidence is kept. Gate
definitions and the test posture rules: sdd-schema §10.]

## Changelog

[See `changelog.md`.]

## Decisions & Open Questions

[See `decisions.md`.]

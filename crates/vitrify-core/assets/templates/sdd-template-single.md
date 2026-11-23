---
sdd: [scope-slug]
title: [Title]
schema_version: 2.2
version: 0.1.0
status: Draft
tdd: ./[path-to-tdd].md
tdd_version: [rev] (Approved, [YYYY-MM-DD])
created: [YYYY-MM-DD]
updated: [YYYY-MM-DD]
scope_roots:
  - [src/scope/]
  - [tests/scope/]
---

> Template for a single-file SDD instance (`specs/<scope>.sdd.md`). Structure only — all rules live in the SDD schema (`vitrify schema show`); the bundle-shape mapping is sdd-schema §1.
>
> ⇄ TDD: the `tdd` / `tdd_version` pin above references [Document Metadata](tdd.md#document-metadata)

## Overview

> ⇄ TDD: derives from [§1 Context & Scope](tdd.md#1-context--scope)

[What is being built. Scope boundary: in and out. Pinned TDD named above is the design authority.]

## Ground Rules

> ⇄ TDD: derives from [§4 Cross-Cutting Concerns](tdd.md#4-cross-cutting-concerns)

[Definitions, normative-language note, and the fixed conventions every implementer obeys — including the test posture: which tests exist before the build, which units write tests, and whether tests precede implementation (sdd-schema §10).]

## Technology Decisions

> ⇄ TDD: derives from [§2 High-Level Architecture](tdd.md#2-high-level-architecture) and [§4 Cross-Cutting Concerns](tdd.md#4-cross-cutting-concerns)

| Dimension | Decision | Pin | Authority |
| --- | --- | --- | --- |
| Language | [name] | [exact version] | TDD §[n] |
| Runtime | [name] | [exact version] | TDD §[n] |
| [Dependency] | [name] | [exact version] | TDD §[n] or D-### |
| Storage | [name] | [exact version] | TDD §[n] |
| Test framework | [name, or none] | [exact version] | D-### |
| Lint / format | [name] | [exact version] | D-### |
| Build system | [name] | [exact version] | D-### |

### Commands

- Bootstrap: `[command]`
- Build:     `[command]`
- Test:      `[command]` [or none — no gate runs it]
- Lint:      `[command]`

### Conventions

- [Imperative convention: naming, error policy, logging shape, directory idioms.]

## System Layout & Ownership

> ⇄ TDD: derives from [§2 High-Level Architecture](tdd.md#2-high-level-architecture)

| Path | Owner | Notes |
| --- | --- | --- |
| [path or glob] | U-001 | [note] |

```
tests:
  - [tests/scope/]
unmanaged:
  - [tool-generated-path/]
```

## Interface Contracts

> ⇄ TDD: skeleton from [§2 High-Level Architecture](tdd.md#2-high-level-architecture); bodies from [§3 Detailed Technical Design](tdd.md#3-detailed-technical-design)

### C-001 — [Name]

- Producer: U-###
- Consumers: U-###
- Stability: Frozen

#### Surface

    [exported types / signatures / endpoint / schema / event / file format]

#### Semantics

- [EARS-form behavior, error codes, ranges, ordering guarantees — exact.]

## Work Units

> ⇄ TDD: derives from [§3 Detailed Technical Design](tdd.md#3-detailed-technical-design)

### U-001 — [Title]

```yaml
unit: U-001
title: [Title]
status: Draft
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: []
parallel_group: —
owns:
  - [path]
consumes: []
built: —
verified: —
trace: —
```

#### Requirements

- **U-001.R1** — [WHEN [trigger],] THE SYSTEM SHALL [observable, testable behavior].

#### Implementation

- [Exact paths, exported signatures, algorithm steps, edge cases, prohibitions.]

#### Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1 | `[command]` | exit 0 |

## Execution Order

> ⇄ TDD: derives from [§5 Execution & Rollout Plan](tdd.md#5-execution--rollout-plan)

```
Phase 0 (foundation): U-001
Phase 1: group A — [P] U-###, U-###
Phase 2: U-###
```

## Verification Gates

> ⇄ TDD: thresholds from [§4 Cross-Cutting Concerns](tdd.md#4-cross-cutting-concerns) and the measurable goals of [§1 Context & Scope](tdd.md#1-context--scope)

[Scope-specific gate thresholds and where build evidence is kept. Gate definitions and the test posture rules: sdd-schema §10.]

## State Ledger

> ⇄ TDD: — no counterpart (implementation state lives only in the SDD)

Progress: 0/[N] Verified

| Unit | Status | Group | Built @ | Verified @ | Note |
| --- | --- | --- | --- | --- | --- |
| U-001 | Draft | — | — | — | |

## Journal

> ⇄ TDD: — no counterpart (append-only history lives only in the SDD)

```
```

## Changelog

> ⇄ TDD: re-pin entries reference [Document Metadata](tdd.md#document-metadata); growth entries realize [§6 Alternatives Considered & Technical Debt](tdd.md#6-alternatives-considered--technical-debt)

### CH-001 — [Title] ([YYYY-MM-DD]) — [old] → [new]

- TDD pin: [unchanged ([rev]) | re-pinned to [rev]]
- Added: [U-###, C-### | —]
- Modified: [… | —]
- Removed: [… | —]
- Staleness applied: [U-### → Stale | —]
- Codebase delta = [build/rebuild list]

## Decisions & Open Questions

> ⇄ TDD: D-### records what [§3 Detailed Technical Design](tdd.md#3-detailed-technical-design) left open; Q-### awaits a new TDD revision; neither may reopen [§6 Alternatives Considered & Technical Debt](tdd.md#6-alternatives-considered--technical-debt)

- **D-001** ([YYYY-MM-DD], SDD [version]) — [TDD is silent on X. Decided: Y. Affects U-###.]
- **Q-001** ([YYYY-MM-DD], open) — [Gap/conflict with TDD §n or C-###. Blocked: U-###.]

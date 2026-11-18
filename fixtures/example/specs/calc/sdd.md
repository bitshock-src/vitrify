---
sdd: calc
title: calc — exact rational expression evaluator
schema_version: 2.0
version: 1.0.0
status: Active
tdd: ../../tdd/calc.md
tdd_version: 1.0 (Approved, 2026-10-28)
created: 2026-10-28
updated: 2026-10-28
scope_roots:
  - calc/
  - tests/
---

# calc — SDD

Bundle-shape SDD instance. Contracts live in `contracts/`, work units in `units/`, the derived
ledger in `ledger.md`, the append-only journal in `journal.ndjson`, change entries in
`changelog.md`, decisions and open questions in `decisions.md`.

## Overview

Build `calc`: a command-line evaluator for integer arithmetic expressions returning exact rational
results (per TDD §1). In scope: a shared error type, a tokenizer, an evaluator, a CLI entry point,
and the pre-authored acceptance suite that proves all of them (TDD §2.2). Out of scope (TDD §1.3):
decimal or scientific literals, variables, functions, a REPL, performance targets, localization.

The pinned TDD above is the design authority; every consequential decision below cites a TDD
section or a `D-###` entry in `decisions.md`.

**The acceptance suite precedes the implementation.** Per TDD §3.4, `tests/` ships with this
specification as part of the U-001 baseline — authored before U-002…U-005 exist — and is the
conformance evidence for every unit. The suite fails (in strict mode) until each unit is built,
and passes when the unit is correct.

## Ground Rules

- The keywords MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are normative in their conventional
  uppercase-only meaning; lowercase forms carry no normative weight.
- Definitions. *Mainline* — the integration branch of record. *Working copy* — an implementer's
  isolated checkout. *Strict mode* — the acceptance-harness mode selected by the environment
  variable `SDD_REQUIRE_IMPL=1` (semantics in §5 Conventions; authority TDD §3.4).
- This SDD is self-contained: every unit is buildable offline from this bundle, the pinned TDD, and
  the repository. External links are provenance, never required reading.
- **The acceptance suite is law.** Every path under `tests/` is owned by U-001, is authored before
  any implementation unit, and is read-only for every other unit (TDD §3.4). A failing acceptance
  test is an implementation defect. An acceptance test believed to be wrong is a specification
  conflict: record a `Q-###` in `decisions.md` and stop — an acceptance test MUST NOT be edited to
  make an implementation pass.
- Error-message prose is non-normative; an error's `code` and `pos` are normative (TDD §3.1).
  Requirements constrain message text only where they say so explicitly.
- Implementers write only the paths their unit owns (§6 System Layout & Ownership) plus the unit's
  own status fields. Needing to write anywhere else means the specification is wrong: stop and
  record a `Q-###`.
- A unit is built from, at most: this region, the Technology Decisions region, the unit's own file,
  and the contracts in its `consumes` list — nothing else.
- Identifiers: units `U-###`, contracts `C-###`, decisions `D-###`, open questions `Q-###`, change
  entries `CH-###`, requirements `U-###.R#`. IDs are assigned once and never reused.

## Technology Decisions

| Dimension | Decision | Pin | Authority |
| --- | --- | --- | --- |
| Language | Python (CPython) | ≥ 3.10, < 4 | TDD §2.2 |
| Runtime | `python3` on PATH (CPython) | ≥ 3.10, enforced by the Bootstrap command | TDD §2.2 |
| Dependencies | none — standard library only | no lockfile: the dependency graph is empty | TDD §1.2, §2.2 |
| Storage | none | — | TDD §4.1 |
| Test framework | `unittest` (standard library) | ships with the pinned runtime | TDD §2.2, §3.4 |
| Lint / format | style gate `tests/test_style.py` run under `unittest` | part of the U-001 baseline | TDD §4.4 |
| Build system | `compileall` (standard-library byte-compilation check) | ships with the pinned runtime | TDD §2.2 |

### Commands

- Bootstrap: `python3 -c "import sys; assert sys.version_info >= (3, 10), sys.version"`
- Build:     `python3 -m compileall -q calc tests`
- Test:      `python3 -m unittest discover -s tests -t .`
- Lint:      `python3 -m unittest tests.test_style`

All four run from the repository root, are non-interactive, and exit 0 in every consistent
repository state — including the shipped pre-implementation baseline, where the Test command
reports the acceptance modules of unbuilt units as *skipped* (TDD §3.4).

### Conventions

- Every failure that crosses a unit boundary is the C-001 error; never raise or leak any other
  exception type across a contract surface (TDD §3.1). The single sanctioned exception: C-001
  construction with an unknown code raises `ValueError`, as C-001 itself specifies.
- Style (TDD §4.4, enforced by the Lint command): 4-space indentation, no tab characters, no
  trailing whitespace, no line longer than 100 columns, in every `.py` file under `calc/` and
  `tests/`.
- Standard library only: no third-party import anywhere under `calc/` or `tests/` (TDD §1.2).
- A module under `calc/` imports another unit's module only where a contract in its unit's
  `consumes` list names that module.
- **Strict mode** (TDD §3.4): a Verification command prefixed with `SDD_REQUIRE_IMPL=1` treats a
  module under test that cannot be imported as a failure; without the variable, such modules are
  reported as skipped (the pre-build state). A module that exists but fails to import is a failure
  in both modes. Commands using the prefix require a POSIX shell.

## System Layout & Ownership

| Path | Owner | Notes |
| --- | --- | --- |
| calc/__init__.py | U-001 | package root; ships with the baseline |
| tests/* | U-001 | pre-authored acceptance suite — read-only law for U-002…U-005 (TDD §3.4) |
| calc/errors.py | U-002 | C-001 implementation |
| calc/tokenizer.py | U-003 | C-002 implementation |
| calc/evaluator.py | U-004 | C-003 implementation |
| calc/__main__.py | U-005 | C-004 implementation |

```
unmanaged:
  - */__pycache__/*
```

## Execution Order

```
Phase 0 (foundation): U-001        # scaffold + acceptance suite; ships Verified in the baseline
Phase 1: U-002                     # shared error type (C-001) — blocks both group-A units
Phase 2: group A — [P] U-003, U-004
Phase 3: U-005                     # CLI: the end-to-end slice
```

U-003 and U-004 are mutually independent by construction: U-004 consumes only the C-002 token
*shape* and never the tokenizer module (TDD §3.1), and the two own disjoint paths. Either may be
claimed first; both may be built simultaneously.

## Verification Gates

- **Unit gate** (working copy; Claimed → Built): §5 Bootstrap, then Build, then every row of the
  unit's Verification table, then Lint — all exit 0, run from the working-copy root.
- **Integration gate** (mainline; Built → Verified): §5 Bootstrap, then Build, then this unit's
  Verification table, then every currently-Verified unit's Verification table, then Lint — all
  exit 0.
- **Write-set check (integration).** Work for a unit MUST be rejected when its changed paths,
  relative to the integration base, include any path outside the unit's `owns` list plus the
  unit's own spec file (`units/<unit>-*.md`). Because no implementation unit owns anything under
  `tests/`, this mechanically protects the acceptance suite (TDD §3.4).
- **Tests precede implementation** (TDD §3.4). Every Verification command of U-002…U-005 runs in
  strict mode and MUST be observed failing before the unit's Implementation block is executed.
- **Contract conformance** is carried by the acceptance suite itself: each contract's behavior is
  asserted by the acceptance modules named in its producer's and consumers' Verification tables,
  which the integration gate already runs.
- Gate evidence: command output is retained in session logs only; the stamps, traces, ledger, and
  journal are the durable record.

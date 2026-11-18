---
unit: U-001
title: Scaffold & acceptance harness (authored baseline)
status: Verified
claimed_by: —
claim_expires: —
claim_seq: 1
depends_on: []
parallel_group: —
owns:
  - calc/__init__.py
  - tests/*
consumes: [C-001, C-002, C-003, C-004]
built: 2026-10-29 @ SDD 1.0.0, commit 0000000
verified: 2026-10-29 @ SDD 1.0.0, commit 0000000
trace:
  unit:   514f1f428d41
  C-001:  ed2b12efed1a
  C-002:  9cd1ad1d1470
  C-003:  31e7ea4d59e5
  C-004:  a7bba1374d4d
  global: b546f01e00cb
---

# U-001 — Scaffold & acceptance harness (authored baseline)

## Requirements

- **U-001.R1** — THE SYSTEM SHALL provide the package `calc` (file `calc/__init__.py`), importable
  from the repository root.
- **U-001.R2** — THE SYSTEM SHALL provide the acceptance modules `tests/test_errors.py`,
  `tests/test_tokenizer.py`, `tests/test_evaluator.py`, and `tests/test_cli.py`, asserting the
  behavior of C-001, C-002, C-003, and C-004 respectively, authored before any of U-002…U-005 is
  implemented (TDD §3.4).
- **U-001.R3** — WHILE strict mode is off, WHEN a module under test is absent, THE SYSTEM SHALL
  report that module's acceptance tests as skipped, not failed.
- **U-001.R4** — WHERE strict mode is on (`SDD_REQUIRE_IMPL=1`), WHEN a module under test is
  absent, THE SYSTEM SHALL report a failure.
- **U-001.R5** — THE SYSTEM SHALL provide `tests/test_style.py`, enforcing the §5 Conventions
  style rules over every `.py` file under `calc/` and `tests/`.

## Implementation

This unit is the authored baseline: its artifacts ship checked in beside the specification
(TDD §3.4, D-002) and are read-only law for every other unit (Ground Rules).

- `calc/__init__.py` — package docstring only; no executable code.
- `tests/__init__.py` — package marker for test discovery.
- `tests/_harness.py` — shared helpers:
  - `import_or_skip(modname)`: import the module; on a `ModuleNotFoundError` naming that module,
    raise `unittest.SkipTest` (strict mode off, R3) or `AssertionError` (strict mode on, R4). Any
    other import failure propagates — a present-but-broken module is never reported as a skip.
  - `require_file(relpath)`: the same skip/fail semantics for file presence (used by the CLI
    acceptance module).
  - `tok` / `num` / `op` / `lparen` / `rparen`: C-002-shaped token factories (shape, not class —
    TDD §3.1) so `tests/test_evaluator.py` runs independently of U-003.
  - `run_cli(*args)`: run the pinned interpreter as a `calc` module subprocess from the repository
    root; return `(exit_code, stdout, stderr)`.
- `tests/test_scaffold.py` — covers R1–R4: package import, acceptance-module presence, and the
  harness's lenient-skip / strict-fail behavior.
- `tests/test_style.py` — covers R5: no tabs, no trailing whitespace, no line over 100 columns.
- Acceptance modules per R2; every test method names the requirement it covers in a one-line
  comment.
- Acceptance modules load units under test only through `_harness.import_or_skip` /
  `_harness.require_file` — never with a plain import statement — so the pre-build state skips
  instead of erroring (TDD §3.4).

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1, R2, R3, R4 | `python3 -m unittest tests.test_scaffold` | exit 0 |
| R5 | `python3 -m unittest tests.test_style` | exit 0 |

Both commands pass in every consistent repository state, including the shipped pre-implementation
baseline.

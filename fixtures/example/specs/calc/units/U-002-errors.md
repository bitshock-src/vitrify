---
unit: U-002
title: Shared error type
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001]
parallel_group: —
owns:
  - calc/errors.py
consumes: [C-001]
built: —
verified: —
trace: —
---

# U-002 — Shared error type

## Requirements

- **U-002.R1** — THE SYSTEM SHALL provide class `CalcError` in `calc/errors.py`, a subclass of
  `Exception`, constructed as `CalcError(code, message, pos=None)`.
- **U-002.R2** — THE SYSTEM SHALL expose the three constructor arguments unchanged as the
  attributes `code`, `message`, and `pos`.
- **U-002.R3** — WHEN a `CalcError` is converted with `str()`, THE SYSTEM SHALL yield exactly the
  `message`.
- **U-002.R4** — THE SYSTEM SHALL export `CODES` as the frozenset of the four C-001 codes; IF
  `CalcError` is constructed with a `code` not in `CODES`, THEN THE SYSTEM SHALL raise
  `ValueError`, not `CalcError`.

## Implementation

- Create `calc/errors.py` implementing C-001 exactly: define `CODES` first, then `CalcError`.
- In the constructor: validate `code` against `CODES` and raise `ValueError` naming the bad code
  (R4); call `super().__init__(message)` so standard exception rendering carries the message (R3);
  then bind the `code`, `message`, and `pos` attributes (R2).
- Export exactly `CODES` and `CalcError`; add nothing else to the module surface (C-001).
- No imports are required; standard library only (§5).
- Tests precede implementation (§10 Verification Gates): run the Verification command, observe it
  fail, implement, observe it pass.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1, R2, R3, R4 | `SDD_REQUIRE_IMPL=1 python3 -m unittest tests.test_errors` | exit 0 |

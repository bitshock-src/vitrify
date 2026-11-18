# C-001 — Shared error type

- Producer: U-002
- Consumers: U-001, U-003, U-004, U-005
- Stability: Frozen (changes require a change entry; MAJOR if breaking)

## Surface

    Module: calc/errors.py  (import as calc.errors)

    CODES = frozenset({"TOKEN_INVALID", "EXPR_EMPTY", "EXPR_SYNTAX", "DIV_ZERO"})

    class CalcError(Exception):
        code: str          # one of CODES
        message: str       # human-readable prose; non-normative (Ground Rules)
        pos: int | None    # 0-based input position; None where no position is meaningful
        def __init__(self, code: str, message: str, pos: int | None = None): ...

The module exports exactly `CODES` and `CalcError`; nothing else.

## Semantics

- IF `CalcError` is constructed with a `code` not in `CODES`, THEN THE SYSTEM SHALL raise
  `ValueError` at construction (a programming error, not an expression error — TDD §3.1).
- WHEN a `CalcError` is converted with `str()`, THE SYSTEM SHALL yield exactly `message`.
- THE SYSTEM SHALL expose the three constructor arguments unchanged as the attributes `code`,
  `message`, and `pos`; `pos` defaults to `None`.
- Code registry (TDD §3.1): `TOKEN_INVALID` is raised only while tokenizing (pos = offending
  character index); `EXPR_EMPTY` (pos None), `EXPR_SYNTAX` (pos = offending token's pos, or None at
  end of input), and `DIV_ZERO` (pos = the offending `/` token's pos) are raised only while
  evaluating.
- Every failure crossing a unit boundary in this scope SHALL be a `CalcError`; no other exception
  type is part of any surface (TDD §3.1).

---
unit: U-003
title: Tokenizer
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001, U-002]
parallel_group: A
owns:
  - calc/tokenizer.py
consumes: [C-001, C-002]
built: —
verified: —
trace: —
---

# U-003 — Tokenizer

## Requirements

- **U-003.R1** — WHEN the input contains a run of ASCII digits, THE SYSTEM SHALL emit exactly one
  `NUM` token whose `value` is the run's base-10 integer — leading zeros permitted (D-001) — and
  whose `pos` is the index of the run's first character.
- **U-003.R2** — WHEN the input contains `+`, `-`, `*`, or `/`, THE SYSTEM SHALL emit one `OP`
  token per character, carrying that character as `value` and its index as `pos`.
- **U-003.R3** — WHEN the input contains `(` or `)`, THE SYSTEM SHALL emit an `LPAREN` or `RPAREN`
  token carrying that character as `value` and its index as `pos`.
- **U-003.R4** — THE SYSTEM SHALL treat space (U+0020) and tab (U+0009) as lexeme separators that
  produce no token.
- **U-003.R5** — IF any other character is encountered, THEN THE SYSTEM SHALL raise the C-001
  error with code `TOKEN_INVALID` and `pos` = that character's index, reading no further input.
- **U-003.R6** — WHEN the input is empty or contains only spaces and tabs, THE SYSTEM SHALL return
  an empty list.

## Implementation

- Create `calc/tokenizer.py` exporting `Token` and `tokenize(text: str) -> list[Token]`, surface
  per C-002.
- `Token`: a `dataclasses.dataclass` with fields `kind: str`, `value`, `pos: int` (C-002 shape).
- Algorithm (TDD §3.3): one forward scan with index `i`. Skip space and tab. On a digit, consume
  the whole run and append `Token("NUM", int(run), start)`. On one of `+ - * /`, append
  `Token("OP", ch, i)`. On `(` or `)`, append `Token("LPAREN" | "RPAREN", ch, i)`. On anything
  else, immediately raise `CalcError("TOKEN_INVALID", <message>, i)` (R5); include the character's
  `repr()` in the message (message prose is otherwise non-normative, Ground Rules).
- Import `CalcError` from `calc.errors` (C-001). No other project import; standard library only
  (§5). Token order is input order (C-002).
- Tests precede implementation (§10 Verification Gates): run the Verification command, observe it
  fail, implement, observe it pass.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1, R2, R3, R4, R5, R6 | `SDD_REQUIRE_IMPL=1 python3 -m unittest tests.test_tokenizer` | exit 0 |

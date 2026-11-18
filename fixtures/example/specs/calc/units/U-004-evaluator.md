---
unit: U-004
title: Evaluator
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001, U-002]
parallel_group: A
owns:
  - calc/evaluator.py
consumes: [C-001, C-002, C-003]
built: —
verified: —
trace: —
---

# U-004 — Evaluator

## Requirements

- **U-004.R1** — WHEN `evaluate` is applied to a grammatical token sequence, THE SYSTEM SHALL
  return a `fractions.Fraction` instance equal to the expression's exact rational value, computed
  without floating-point arithmetic.
- **U-004.R2** — THE SYSTEM SHALL bind `*` and `/` tighter than `+` and `-`.
- **U-004.R3** — THE SYSTEM SHALL group operators of equal precedence left to right.
- **U-004.R4** — THE SYSTEM SHALL evaluate a parenthesized expression as a single factor.
- **U-004.R5** — THE SYSTEM SHALL evaluate `-` before a factor as exact negation of that factor,
  including when nested.
- **U-004.R6** — WHEN the token sequence is empty, THE SYSTEM SHALL raise the C-001 error with
  code `EXPR_EMPTY` and `pos = None`.
- **U-004.R7** — IF the token sequence violates the grammar, THEN THE SYSTEM SHALL raise the C-001
  error with code `EXPR_SYNTAX` and `pos` = the offending token's `pos` — or `pos = None` when the
  sequence ends before a production completes. A token remaining after a complete expression is
  itself an offending token.
- **U-004.R8** — WHEN a division's right operand evaluates to zero, THE SYSTEM SHALL raise the
  C-001 error with code `DIV_ZERO` and `pos` = that `/` token's `pos`, before dividing.

## Implementation

- Create `calc/evaluator.py` exporting `evaluate(tokens) -> fractions.Fraction`, surface and
  grammar per C-003.
- Recursive descent (TDD §3.3): one function per production (`expr`, `term`, `factor`), threading
  a token index. `evaluate`: raise `EXPR_EMPTY` on an empty sequence (R6); parse `expr`; if any
  token remains, raise `EXPR_SYNTAX` at the first remaining token's `pos` (R7); return the value.
- `factor`: on `OP "-"`, recurse and negate (R5); on `NUM`, return `Fraction(value)`; on `LPAREN`,
  parse `expr` and require `RPAREN`; on any other token, raise `EXPR_SYNTAX` at its `pos`; at end
  of input, raise `EXPR_SYNTAX` with `pos = None` (R7).
- Division: before dividing, IF the right operand equals zero, raise `DIV_ZERO` carrying the `/`
  token's `pos` (R8).
- Consume tokens by shape only — read `.kind`, `.value`, `.pos` (C-002, TDD §3.1). Do not import
  `calc.tokenizer`; do not test token class identity.
- Imports: `CalcError` from `calc.errors` (C-001) and `Fraction` from `fractions`; nothing else
  (§5).
- Do not execute or evaluate input as code (TDD §6.1).
- Tests precede implementation (§10 Verification Gates): run the Verification command, observe it
  fail, implement, observe it pass.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1, R2, R3, R4, R5, R6, R7, R8 | `SDD_REQUIRE_IMPL=1 python3 -m unittest tests.test_evaluator` | exit 0 |

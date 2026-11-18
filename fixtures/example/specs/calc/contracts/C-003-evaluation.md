# C-003 — Expression evaluation

- Producer: U-004
- Consumers: U-001, U-005
- Stability: Frozen (changes require a change entry; MAJOR if breaking)

## Surface

    Module: calc/evaluator.py  (import as calc.evaluator)

    def evaluate(tokens) -> fractions.Fraction: ...
        # tokens: any sequence of C-002-shaped tokens (shape, not class — TDD §3.1)

## Semantics

Grammar (TDD §3.3) — binary operators are left-associative; `*` and `/` bind tighter than
`+` and `-`:

    expr   ::= term (("+" | "-") term)*
    term   ::= factor (("*" | "/") factor)*
    factor ::= "-" factor | NUM | "(" expr ")"

- WHEN `tokens` is a grammatical sequence, `evaluate` SHALL return a `fractions.Fraction`
  instance equal to the expression's exact rational value; floating-point arithmetic SHALL NOT
  be used (TDD §1.2).
- `-` before a factor is exact negation and nests (`--3` evaluates to 3).
- WHEN `tokens` is empty, `evaluate` SHALL raise the C-001 error with code `EXPR_EMPTY` and
  `pos = None`.
- IF a token not permitted by the grammar is encountered — including any token remaining after a
  complete `expr` — THEN `evaluate` SHALL raise the C-001 error with code `EXPR_SYNTAX` and
  `pos` = that token's `pos`.
- IF the sequence ends before a production completes, THEN `evaluate` SHALL raise the C-001 error
  with code `EXPR_SYNTAX` and `pos = None`.
- WHEN a division's right operand evaluates to zero, `evaluate` SHALL raise the C-001 error with
  code `DIV_ZERO` and `pos` = that `/` token's `pos`, before performing the division.

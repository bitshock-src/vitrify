---
unit: U-005
title: Command-line entry point
status: Ready
claimed_by: —
claim_expires: —
claim_seq: 0
depends_on: [U-001, U-002, U-003, U-004]
parallel_group: —
owns:
  - calc/__main__.py
consumes: [C-001, C-002, C-003, C-004]
built: —
verified: —
trace: —
---

# U-005 — Command-line entry point

## Requirements

- **U-005.R1** — WHEN invoked with exactly one argument whose expression evaluates, THE SYSTEM
  SHALL print the canonical result and a newline to stdout, print nothing to stderr, and exit 0.
- **U-005.R2** — THE SYSTEM SHALL print results canonically (C-004): WHEN the denominator is 1,
  the base-10 integer with a leading `-` when negative; otherwise `<numerator>/<denominator>` in
  lowest terms with a positive denominator and the sign on the numerator.
- **U-005.R3** — IF the argument count differs from one, THEN THE SYSTEM SHALL print exactly
  `usage: python3 -m calc "EXPR"` and a newline to stderr, print nothing to stdout, and exit 2.
- **U-005.R4** — IF tokenizing or evaluating raises the C-001 error, THEN THE SYSTEM SHALL print
  `error[<code>]: <message>` — plus ` (at position <pos>)` when `pos` is not None — and a newline
  to stderr, print nothing to stdout, and exit 3.
- **U-005.R5** — THE SYSTEM SHALL treat the single argument verbatim, including when it begins
  with `-`.

## Implementation

- Create `calc/__main__.py` implementing C-004: define `main(argv) -> int` taking the argument
  list after the program name, and end the module with
  `if __name__ == "__main__": sys.exit(main(sys.argv[1:]))`.
- Inspect `argv` directly; do not use an argument-parsing library — flag parsing conflicts with
  leading-`-` expressions (TDD §6.1, R5). `len(argv) != 1` → usage path (R3).
- Pipeline: `tokenize` (C-002), then `evaluate` (C-003), inside one `try` block catching only the
  C-001 error (R4). Let any other exception propagate.
- Format per R2: when `result.denominator == 1`, print `result.numerator`; otherwise print
  `f"{result.numerator}/{result.denominator}"` — `fractions.Fraction` already normalizes sign and
  lowest terms (C-003).
- Write results with `print(...)` and diagnostics with `print(..., file=sys.stderr)`; never mix
  the streams (R1, R3, R4).
- Imports: `sys`, `calc.errors` (C-001), `calc.tokenizer` (C-002), `calc.evaluator` (C-003);
  nothing else (§5).
- Tests precede implementation (§10 Verification Gates): run the first Verification command,
  observe it fail, implement, observe it pass.

## Verification

| Covers | Command | Must satisfy |
| --- | --- | --- |
| R1, R2, R3, R4, R5 | `SDD_REQUIRE_IMPL=1 python3 -m unittest tests.test_cli` | exit 0 |
| end-to-end slice (all units) | `SDD_REQUIRE_IMPL=1 python3 -m unittest discover -s tests -t .` | exit 0 |

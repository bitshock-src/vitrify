# C-004 — Command-line surface

- Producer: U-005
- Consumers: U-001
- Stability: Frozen (changes require a change entry; MAJOR if breaking)

## Surface

    Module: calc/__main__.py   (runnable as: python3 -m calc "EXPR")

    One positional argument: the expression text, taken verbatim — it may begin with "-";
    no flag parsing exists (TDD §6.1).

## Semantics

| Condition | stdout | stderr | exit |
| --- | --- | --- | --- |
| the expression evaluates | canonical result + newline | empty | 0 |
| argument count ≠ 1 | empty | `usage: python3 -m calc "EXPR"` + newline | 2 |
| the C-001 error is raised | empty | error line (below) + newline | 3 |

- **Canonical result** (TDD §3.2): WHEN the result's denominator is 1, THE SYSTEM SHALL print the
  base-10 integer with a leading `-` when negative; otherwise it SHALL print
  `<numerator>/<denominator>` in lowest terms with a positive denominator and the sign on the
  numerator (e.g. `17/2`, `-7/2`).
- **Error line** (TDD §3.2): `error[<code>]: <message>`, followed by ` (at position <pos>)` when
  `pos` is not None. Message prose is non-normative (Ground Rules); code and position are.
- **Usage line**: exactly `usage: python3 -m calc "EXPR"`.
- THE SYSTEM SHALL write results only to stdout and diagnostics only to stderr; the streams are
  never mixed.

# C-002 — Token stream

- Producer: U-003
- Consumers: U-001, U-004, U-005
- Stability: Frozen (changes require a change entry; MAJOR if breaking)

## Surface

    Module: calc/tokenizer.py  (import as calc.tokenizer)

    class Token:          # concrete token class; constructed only by the tokenizer
        kind: str         # "NUM" | "OP" | "LPAREN" | "RPAREN"
        value: int | str  # base-10 int when kind == "NUM";
                          # the literal character "+" "-" "*" "/" "(" ")" otherwise
        pos: int          # 0-based index of the lexeme's first character

    def tokenize(text: str) -> list[Token]: ...

## Semantics

- **Structural typing** (TDD §3.1): consumers SHALL depend only on the attribute shape
  (`kind`, `value`, `pos`), never on the class identity. Any object exposing those three
  attributes with the domains above is a valid token for every consumer in this scope.
- WHEN `text` contains a run of ASCII digits `[0-9]+`, `tokenize` SHALL emit exactly one `NUM`
  token whose `value` is the run's base-10 integer — leading zeros permitted (D-001) — and whose
  `pos` is the index of the run's first character.
- WHEN `text` contains `+`, `-`, `*`, or `/`, `tokenize` SHALL emit one `OP` token per character,
  carrying that character as `value`.
- WHEN `text` contains `(` or `)`, `tokenize` SHALL emit an `LPAREN` or `RPAREN` token carrying
  that character as `value`.
- THE SYSTEM SHALL treat space (U+0020) and tab (U+0009) as lexeme separators that produce no
  token; no other character is whitespace.
- IF any other character is encountered, THEN `tokenize` SHALL raise the C-001 error with code
  `TOKEN_INVALID` and `pos` = that character's index, reading no further input.
- WHEN `text` is empty or contains only spaces and tabs, `tokenize` SHALL return `[]`.
- Tokens SHALL appear in input order.

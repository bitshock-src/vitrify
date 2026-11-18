"""U-003 acceptance — the tokenizer (C-002)."""
import unittest

from tests import _harness

errors = _harness.import_or_skip("calc.errors")
tokenizer = _harness.import_or_skip("calc.tokenizer")


def shapes(tokens):
    # Tokens are compared by shape, not class (C-002 / TDD §3.1).
    return [(t.kind, t.value, t.pos) for t in tokens]


class TestTokenize(unittest.TestCase):
    def test_single_number(self):
        # U-003.R1 — a digit run is one NUM token with its base-10 value.
        self.assertEqual(shapes(tokenizer.tokenize("42")), [("NUM", 42, 0)])

    def test_leading_zeros(self):
        # U-003.R1 / D-001 — leading zeros are permitted: "007" is 7.
        self.assertEqual(shapes(tokenizer.tokenize("007")), [("NUM", 7, 0)])

    def test_operators(self):
        # U-003.R2 — each of + - * / is one OP token carrying the character.
        self.assertEqual(shapes(tokenizer.tokenize("+-*/")),
                         [("OP", "+", 0), ("OP", "-", 1),
                          ("OP", "*", 2), ("OP", "/", 3)])

    def test_parens(self):
        # U-003.R3 — parentheses tokenize as LPAREN / RPAREN.
        self.assertEqual(shapes(tokenizer.tokenize("()")),
                         [("LPAREN", "(", 0), ("RPAREN", ")", 1)])

    def test_whitespace_skipped_positions_kept(self):
        # U-003.R4 + R1/R2 — spaces and tabs emit nothing; pos is the lexeme start.
        self.assertEqual(shapes(tokenizer.tokenize(" 1 +\t23 ")),
                         [("NUM", 1, 1), ("OP", "+", 3), ("NUM", 23, 5)])

    def test_invalid_character(self):
        # U-003.R5 — an unsupported character raises TOKEN_INVALID at its index.
        with self.assertRaises(errors.CalcError) as ctx:
            tokenizer.tokenize("1+$2")
        self.assertEqual(ctx.exception.code, "TOKEN_INVALID")
        self.assertEqual(ctx.exception.pos, 2)

    def test_empty_and_blank(self):
        # U-003.R6 — empty or whitespace-only input yields no tokens.
        self.assertEqual(tokenizer.tokenize(""), [])
        self.assertEqual(tokenizer.tokenize(" \t "), [])

    def test_expression_token_order(self):
        # U-003.R1–R4 / C-002 — tokens appear in input order.
        self.assertEqual(shapes(tokenizer.tokenize("(1+20)*3")),
                         [("LPAREN", "(", 0), ("NUM", 1, 1), ("OP", "+", 2),
                          ("NUM", 20, 3), ("RPAREN", ")", 5), ("OP", "*", 6),
                          ("NUM", 3, 7)])

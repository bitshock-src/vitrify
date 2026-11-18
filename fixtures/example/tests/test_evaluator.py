"""U-004 acceptance — the evaluator (C-003). Tokens here are built by shape
(TDD §3.1), so this module never imports the tokenizer and runs — and can be
gated — independently of U-003."""
import unittest
from fractions import Fraction

from tests import _harness
from tests._harness import lparen, num, op, rparen

errors = _harness.import_or_skip("calc.errors")
evaluator = _harness.import_or_skip("calc.evaluator")


class TestEvaluate(unittest.TestCase):
    def test_returns_fraction(self):
        # U-004.R1 — the result is a fractions.Fraction instance.
        r = evaluator.evaluate([num(3, 0)])
        self.assertIsInstance(r, Fraction)
        self.assertEqual(r, Fraction(3))

    def test_exact_division(self):
        # U-004.R1 — division is exact rational arithmetic: 1/3 is one third.
        r = evaluator.evaluate([num(1, 0), op("/", 1), num(3, 2)])
        self.assertEqual(r, Fraction(1, 3))

    def test_precedence(self):
        # U-004.R2 — "*" binds tighter than "+": 2+3*4 = 14.
        toks = [num(2, 0), op("+", 1), num(3, 2), op("*", 3), num(4, 4)]
        self.assertEqual(evaluator.evaluate(toks), Fraction(14))

    def test_left_associativity_minus(self):
        # U-004.R3 — 8-3-2 groups as (8-3)-2 = 3.
        toks = [num(8, 0), op("-", 1), num(3, 2), op("-", 3), num(2, 4)]
        self.assertEqual(evaluator.evaluate(toks), Fraction(3))

    def test_left_associativity_divide(self):
        # U-004.R3 — 16/4/2 groups as (16/4)/2 = 2.
        toks = [num(16, 0), op("/", 2), num(4, 3), op("/", 4), num(2, 5)]
        self.assertEqual(evaluator.evaluate(toks), Fraction(2))

    def test_parentheses_override(self):
        # U-004.R4 — (2+3)*4 = 20.
        toks = [lparen(0), num(2, 1), op("+", 2), num(3, 3), rparen(4),
                op("*", 5), num(4, 6)]
        self.assertEqual(evaluator.evaluate(toks), Fraction(20))

    def test_unary_minus(self):
        # U-004.R5 — -2*3 = -6: unary minus is a factor, so it binds to 2.
        toks = [op("-", 0), num(2, 1), op("*", 2), num(3, 3)]
        self.assertEqual(evaluator.evaluate(toks), Fraction(-6))

    def test_nested_unary_minus(self):
        # U-004.R5 — --3 = 3: negation nests.
        toks = [op("-", 0), op("-", 1), num(3, 2)]
        self.assertEqual(evaluator.evaluate(toks), Fraction(3))

    def test_empty_input(self):
        # U-004.R6 — an empty token sequence raises EXPR_EMPTY with pos None.
        with self.assertRaises(errors.CalcError) as ctx:
            evaluator.evaluate([])
        self.assertEqual(ctx.exception.code, "EXPR_EMPTY")
        self.assertIsNone(ctx.exception.pos)

    def test_unexpected_token(self):
        # U-004.R7 — "1 2": the second NUM is unexpected; pos names it.
        with self.assertRaises(errors.CalcError) as ctx:
            evaluator.evaluate([num(1, 0), num(2, 2)])
        self.assertEqual(ctx.exception.code, "EXPR_SYNTAX")
        self.assertEqual(ctx.exception.pos, 2)

    def test_unexpected_operator(self):
        # U-004.R7 — "1+*2": "*" cannot start a factor; pos names it.
        toks = [num(1, 0), op("+", 1), op("*", 2), num(2, 3)]
        with self.assertRaises(errors.CalcError) as ctx:
            evaluator.evaluate(toks)
        self.assertEqual(ctx.exception.code, "EXPR_SYNTAX")
        self.assertEqual(ctx.exception.pos, 2)

    def test_truncated_input(self):
        # U-004.R7 — "1+" ends mid-production; pos is None.
        with self.assertRaises(errors.CalcError) as ctx:
            evaluator.evaluate([num(1, 0), op("+", 1)])
        self.assertEqual(ctx.exception.code, "EXPR_SYNTAX")
        self.assertIsNone(ctx.exception.pos)

    def test_empty_parens(self):
        # U-004.R7 — "()": RPAREN cannot start a factor; pos = 1.
        with self.assertRaises(errors.CalcError) as ctx:
            evaluator.evaluate([lparen(0), rparen(1)])
        self.assertEqual(ctx.exception.code, "EXPR_SYNTAX")
        self.assertEqual(ctx.exception.pos, 1)

    def test_division_by_zero(self):
        # U-004.R8 — "1/(2-2)": zero divisor raises DIV_ZERO at the "/" pos.
        toks = [num(1, 0), op("/", 1), lparen(2), num(2, 3), op("-", 4),
                num(2, 5), rparen(6)]
        with self.assertRaises(errors.CalcError) as ctx:
            evaluator.evaluate(toks)
        self.assertEqual(ctx.exception.code, "DIV_ZERO")
        self.assertEqual(ctx.exception.pos, 1)

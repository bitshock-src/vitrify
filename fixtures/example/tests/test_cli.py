"""U-005 acceptance — the command-line surface (C-004), exercised end to end
through a real interpreter subprocess (the walking skeleton of the scope)."""
import unittest

from tests import _harness

_harness.require_file("calc/__main__.py")


class TestCli(unittest.TestCase):
    def test_integer_result(self):
        # U-005.R1, R2 — denominator 1 prints as a plain integer.
        code, out, err = _harness.run_cli("1+2*3")
        self.assertEqual((code, out, err), (0, "7\n", ""))

    def test_fraction_result(self):
        # U-005.R2 — non-integral results print as num/den in lowest terms.
        code, out, err = _harness.run_cli("(1+2)*3-4/8")
        self.assertEqual((code, out, err), (0, "17/2\n", ""))

    def test_negative_fraction(self):
        # U-005.R2 — the sign rides on the numerator.
        code, out, err = _harness.run_cli("-(7/2)")
        self.assertEqual((code, out, err), (0, "-7/2\n", ""))

    def test_leading_minus_argument(self):
        # U-005.R5 — an argument beginning with "-" is an expression, not a flag.
        code, out, err = _harness.run_cli("-5+2")
        self.assertEqual((code, out, err), (0, "-3\n", ""))

    def test_usage_no_args(self):
        # U-005.R3 — no argument: the exact usage line on stderr, exit 2.
        code, out, err = _harness.run_cli()
        self.assertEqual((code, out), (2, ""))
        self.assertEqual(err, 'usage: python3 -m calc "EXPR"\n')

    def test_usage_two_args(self):
        # U-005.R3 — more than one argument: the exact usage line, exit 2.
        code, out, err = _harness.run_cli("1", "2")
        self.assertEqual((code, out), (2, ""))
        self.assertEqual(err, 'usage: python3 -m calc "EXPR"\n')

    def test_error_with_position(self):
        # U-005.R4 — calc errors: error[CODE]: message (at position N), exit 3.
        code, out, err = _harness.run_cli("1/0")
        self.assertEqual((code, out), (3, ""))
        self.assertRegex(err, r"^error\[DIV_ZERO\]: .+ \(at position 1\)\n$")

    def test_error_without_position(self):
        # U-005.R4 — a position-less error omits the position suffix.
        code, out, err = _harness.run_cli("")
        self.assertEqual((code, out), (3, ""))
        self.assertRegex(err, r"^error\[EXPR_EMPTY\]: .+\n$")
        self.assertNotIn("(at position", err)

    def test_invalid_character_error(self):
        # U-005.R4 — tokenizer errors surface identically: "^" at position 1.
        code, out, err = _harness.run_cli("2^3")
        self.assertEqual((code, out), (3, ""))
        self.assertRegex(err, r"^error\[TOKEN_INVALID\]: .+ \(at position 1\)\n$")

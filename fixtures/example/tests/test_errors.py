"""U-002 acceptance — the shared error type (C-001)."""
import unittest

from tests import _harness

errors = _harness.import_or_skip("calc.errors")


class TestCalcError(unittest.TestCase):
    def test_is_exception_with_default_pos(self):
        # U-002.R1 — CalcError(code, message) is an Exception; pos defaults to None.
        e = errors.CalcError("DIV_ZERO", "division by zero")
        self.assertIsInstance(e, Exception)
        self.assertIsNone(e.pos)

    def test_attributes(self):
        # U-002.R2 — .code / .message / .pos expose the constructor arguments.
        e = errors.CalcError("TOKEN_INVALID", "bad character", 7)
        self.assertEqual(e.code, "TOKEN_INVALID")
        self.assertEqual(e.message, "bad character")
        self.assertEqual(e.pos, 7)

    def test_str_is_message(self):
        # U-002.R3 — str(e) is exactly the message.
        e = errors.CalcError("EXPR_EMPTY", "nothing to evaluate")
        self.assertEqual(str(e), "nothing to evaluate")

    def test_codes_registry(self):
        # U-002.R4 — CODES is the frozen registry of the four C-001 codes.
        self.assertEqual(errors.CODES, frozenset(
            {"TOKEN_INVALID", "EXPR_EMPTY", "EXPR_SYNTAX", "DIV_ZERO"}))

    def test_unknown_code_rejected(self):
        # U-002.R4 — a code outside CODES raises ValueError at construction.
        with self.assertRaises(ValueError):
            errors.CalcError("NOT_A_CODE", "boom")

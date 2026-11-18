"""U-001 — scaffold & harness self-checks. Passes in every consistent repository
state, including the shipped pre-implementation baseline."""
import importlib
import os
import unittest

from tests import _harness

ACCEPTANCE_MODULES = (
    "tests/test_errors.py",      # U-002 / C-001
    "tests/test_tokenizer.py",   # U-003 / C-002
    "tests/test_evaluator.py",   # U-004 / C-003
    "tests/test_cli.py",         # U-005 / C-004
)


class TestScaffold(unittest.TestCase):
    def test_calc_package_importable(self):
        # U-001.R1 — the calc package exists and imports from the repository root.
        mod = importlib.import_module("calc")
        self.assertTrue(hasattr(mod, "__file__"))

    def test_acceptance_modules_present(self):
        # U-001.R2 — the pre-authored acceptance suite is complete.
        for rel in ACCEPTANCE_MODULES:
            self.assertTrue((_harness.ROOT / rel).is_file(), rel + " is missing")

    def test_style_gate_present(self):
        # U-001.R5 — the style gate exists.
        self.assertTrue((_harness.ROOT / "tests" / "test_style.py").is_file())

    def test_lenient_mode_skips_missing_module(self):
        # U-001.R3 — with strict mode off, an absent module under test skips.
        old = os.environ.pop(_harness.STRICT_ENV, None)
        try:
            with self.assertRaises(unittest.SkipTest):
                _harness.import_or_skip("calc.does_not_exist_fixture")
        finally:
            if old is not None:
                os.environ[_harness.STRICT_ENV] = old

    def test_strict_mode_fails_missing_module(self):
        # U-001.R4 — with SDD_REQUIRE_IMPL=1, an absent module under test fails.
        old = os.environ.get(_harness.STRICT_ENV)
        os.environ[_harness.STRICT_ENV] = "1"
        try:
            with self.assertRaises(AssertionError):
                _harness.import_or_skip("calc.does_not_exist_fixture")
        finally:
            if old is None:
                del os.environ[_harness.STRICT_ENV]
            else:
                os.environ[_harness.STRICT_ENV] = old

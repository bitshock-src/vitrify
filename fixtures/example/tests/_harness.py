"""Shared helpers for the calc acceptance suite (owned by U-001; see TDD §3.4).

Lenient vs strict mode (U-001.R3 / U-001.R4): a module under test that is not
yet built reports as *skipped* by default, so the whole suite runs cleanly in
the pre-implementation state; with SDD_REQUIRE_IMPL=1 the same absence is a
*failure*, so no per-unit verification gate can pass by omission. A module
that exists but cannot be imported is a failure in both modes.
"""
import importlib
import os
import subprocess
import sys
import unittest
from collections import namedtuple
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

STRICT_ENV = "SDD_REQUIRE_IMPL"


def strict():
    return os.environ.get(STRICT_ENV) == "1"


def import_or_skip(modname):
    """Import a module under test, honoring lenient/strict mode (U-001.R3, R4)."""
    try:
        return importlib.import_module(modname)
    except ModuleNotFoundError as exc:
        if exc.name != modname and not modname.startswith(exc.name + "."):
            raise  # some *other* module is missing inside it: a real defect, never a skip
        if strict():
            raise AssertionError(
                "strict mode: required module not importable: " + modname) from exc
        raise unittest.SkipTest(modname + " not built yet") from exc


def require_file(relpath):
    """File-presence twin of import_or_skip, for surfaces exercised by subprocess."""
    if (ROOT / relpath).exists():
        return
    if strict():
        raise AssertionError("strict mode: required file missing: " + relpath)
    raise unittest.SkipTest(relpath + " not built yet")


# C-002-shaped tokens built by shape, not class (TDD §3.1), so the evaluator's
# acceptance tests run with no dependency on the tokenizer unit.
FakeToken = namedtuple("FakeToken", ["kind", "value", "pos"])


def tok(kind, value, pos):
    return FakeToken(kind, value, pos)


def num(value, pos):
    return tok("NUM", value, pos)


def op(ch, pos):
    return tok("OP", ch, pos)


def lparen(pos):
    return tok("LPAREN", "(", pos)


def rparen(pos):
    return tok("RPAREN", ")", pos)


def run_cli(*args):
    """Run the CLI (C-004) as a real interpreter subprocess from the repo root."""
    proc = subprocess.run([sys.executable, "-m", "calc", *args], cwd=ROOT,
                          capture_output=True, text=True, timeout=60)
    return proc.returncode, proc.stdout, proc.stderr

"""U-001 — mechanical style gate over calc/ and tests/ (the §5 Lint command;
rules per §5 Conventions / TDD §4.4)."""
import unittest

from tests import _harness


def python_files():
    for d in ("calc", "tests"):
        base = _harness.ROOT / d
        if base.is_dir():
            for path in sorted(base.rglob("*.py")):
                if "__pycache__" not in path.parts:
                    yield path


class TestStyle(unittest.TestCase):
    def test_style_rules(self):
        # U-001.R5 — no tabs, no trailing whitespace, no line over 100 columns.
        problems = []
        for path in python_files():
            rel = path.relative_to(_harness.ROOT)
            text = path.read_text(encoding="utf-8")
            for n, line in enumerate(text.splitlines(), 1):
                if "\t" in line:
                    problems.append("%s:%d: tab character" % (rel, n))
                if line != line.rstrip():
                    problems.append("%s:%d: trailing whitespace" % (rel, n))
                if len(line) > 100:
                    problems.append("%s:%d: line longer than 100 columns" % (rel, n))
        self.assertEqual(problems, [])

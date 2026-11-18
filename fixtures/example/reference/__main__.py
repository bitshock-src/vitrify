"""Command-line entry point (C-004): python3 -m calc "EXPR"."""
import sys

from calc.errors import CalcError
from calc.evaluator import evaluate
from calc.tokenizer import tokenize

USAGE = 'usage: python3 -m calc "EXPR"'


def main(argv):
    if len(argv) != 1:  # U-005.R3
        print(USAGE, file=sys.stderr)
        return 2
    try:
        result = evaluate(tokenize(argv[0]))  # U-005.R1, R5 — verbatim argument
    except CalcError as exc:  # U-005.R4
        line = "error[" + exc.code + "]: " + exc.message
        if exc.pos is not None:
            line += " (at position " + str(exc.pos) + ")"
        print(line, file=sys.stderr)
        return 3
    if result.denominator == 1:  # U-005.R2 — canonical form
        print(result.numerator)
    else:
        print("%d/%d" % (result.numerator, result.denominator))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

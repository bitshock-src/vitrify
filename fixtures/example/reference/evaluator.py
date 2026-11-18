"""Evaluator (C-003): a C-002-shaped token sequence to its exact rational value.

Tokens are consumed by shape only (kind/value/pos — TDD §3.1); this module
never imports the tokenizer.
"""
from fractions import Fraction

from calc.errors import CalcError


def evaluate(tokens):
    toks = list(tokens)
    if not toks:  # U-004.R6
        raise CalcError("EXPR_EMPTY", "nothing to evaluate")
    value, i = _expr(toks, 0)
    if i < len(toks):  # U-004.R7 — trailing token is offending
        raise CalcError("EXPR_SYNTAX", "unexpected token", toks[i].pos)
    return value


def _expr(toks, i):
    value, i = _term(toks, i)
    while i < len(toks) and toks[i].kind == "OP" and toks[i].value in "+-":
        op = toks[i]  # U-004.R3 — left associativity by iteration
        right, i = _term(toks, i + 1)
        value = value + right if op.value == "+" else value - right
    return value, i


def _term(toks, i):
    value, i = _factor(toks, i)
    while i < len(toks) and toks[i].kind == "OP" and toks[i].value in "*/":
        op = toks[i]  # U-004.R2 — tighter binding than +/-
        right, i = _factor(toks, i + 1)
        if op.value == "/":
            if right == 0:  # U-004.R8 — check before dividing
                raise CalcError("DIV_ZERO", "division by zero", op.pos)
            value = value / right
        else:
            value = value * right
    return value, i


def _factor(toks, i):
    if i >= len(toks):  # U-004.R7 — end of input: pos None
        raise CalcError("EXPR_SYNTAX", "unexpected end of input")
    t = toks[i]
    if t.kind == "OP" and t.value == "-":  # U-004.R5 — negation nests
        value, i = _factor(toks, i + 1)
        return -value, i
    if t.kind == "NUM":  # U-004.R1 — exact rational arithmetic
        return Fraction(t.value), i + 1
    if t.kind == "LPAREN":  # U-004.R4
        value, i = _expr(toks, i + 1)
        if i >= len(toks):
            raise CalcError("EXPR_SYNTAX", "unexpected end of input")
        if toks[i].kind != "RPAREN":
            raise CalcError("EXPR_SYNTAX", "expected )", toks[i].pos)
        return value, i + 1
    raise CalcError("EXPR_SYNTAX", "unexpected token", t.pos)  # U-004.R7

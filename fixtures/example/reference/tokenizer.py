"""Tokenizer (C-002): expression text to a token list."""
from dataclasses import dataclass

from calc.errors import CalcError

_DIGITS = "0123456789"
_OPS = "+-*/"


@dataclass
class Token:
    kind: str
    value: object
    pos: int


def tokenize(text):
    tokens = []
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch in " \t":  # U-003.R4 — separators, no token
            i += 1
        elif ch in _DIGITS:  # U-003.R1 — one NUM per digit run
            start = i
            while i < n and text[i] in _DIGITS:
                i += 1
            tokens.append(Token("NUM", int(text[start:i]), start))
        elif ch in _OPS:  # U-003.R2
            tokens.append(Token("OP", ch, i))
            i += 1
        elif ch == "(":  # U-003.R3
            tokens.append(Token("LPAREN", ch, i))
            i += 1
        elif ch == ")":  # U-003.R3
            tokens.append(Token("RPAREN", ch, i))
            i += 1
        else:  # U-003.R5 — abort immediately at the offending index
            raise CalcError("TOKEN_INVALID", "invalid character " + repr(ch), i)
    return tokens  # U-003.R6 — [] for empty/blank input

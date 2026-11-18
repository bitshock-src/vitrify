"""Shared error type (C-001)."""

CODES = frozenset({"TOKEN_INVALID", "EXPR_EMPTY", "EXPR_SYNTAX", "DIV_ZERO"})


class CalcError(Exception):
    """The single error type crossing calc unit boundaries (C-001)."""

    def __init__(self, code, message, pos=None):
        if code not in CODES:  # U-002.R4
            raise ValueError("unknown error code: " + repr(code))
        super().__init__(message)  # U-002.R3
        self.code = code  # U-002.R2
        self.message = message
        self.pos = pos  # U-002.R1 — defaults to None

"""Calculations typed into number fields, as egui's ``crates/ui-egui/src/calc.rs`` (#817):
``100*2``, ``1920/3``, ``(50+10)*1.5``, and, applied to the field's current value, ``*2`` or
``/3``. Reads ``+ - * /`` (also ``×`` and ``÷``), brackets, decimals and signs; a unit (letters,
``%``, ``°``) and spaces are ignored. Anything else gives ``None``, never an exception: the text
is capped, nesting is bounded, and only finite results come back."""

from __future__ import annotations

import math

MAX_LEN = 256  # longest text read (characters)
MAX_DEPTH = 64  # deepest nesting of brackets and signs


def _skipped(c: str) -> bool:
    return c.isalpha() or c.isspace() or c in "%°"


def evaluate(text: str, current: float) -> float | None:
    """The value of ``text`` typed into a field that holds ``current``. A text starting with
    ``*`` or ``/`` applies to ``current`` (``*2`` doubles it); ``+`` and ``-`` at the start are signs."""
    if len(text) > MAX_LEN:
        return None
    # spaces or a unit between two numbers (`1e3`, `10x20`, `1920 1080`) would fuse them into one
    # number once dropped: refuse rather than guess
    after_number = gap = False
    for c in text:
        if _skipped(c):
            gap = True
            continue
        number_char = c.isascii() and (c.isdigit() or c == ".")
        if number_char and after_number and gap:
            return None
        after_number, gap = number_char, False
    chars = [{"×": "*", "÷": "/"}.get(c, c) for c in text if not _skipped(c)]
    p = _Parser(chars)
    try:
        if chars[:1] in (["*"], ["/"]):
            if not math.isfinite(current):
                return None
            v = p.expr_from(p.term_from(current, 0), 0)
        else:
            v = p.expr(0)
    except _Fail:
        return None
    return v if p.pos == len(chars) and math.isfinite(v) else None


class _Fail(Exception):
    pass


def _finite(v: float) -> float:
    if not math.isfinite(v):
        raise _Fail
    return v


class _Parser:
    def __init__(self, chars: list[str]) -> None:
        self.chars = chars
        self.pos = 0

    def peek(self) -> str | None:
        return self.chars[self.pos] if self.pos < len(self.chars) else None

    def expr(self, depth: int) -> float:  # term (('+' | '-') term)*
        return self.expr_from(self.term(depth), depth)

    def expr_from(self, v: float, depth: int) -> float:
        while (op := self.peek()) in ("+", "-"):
            self.pos += 1
            r = self.term(depth)
            v = _finite(v + r if op == "+" else v - r)
        return v

    def term(self, depth: int) -> float:  # factor (('*' | '/') factor)*
        return self.term_from(self.factor(depth), depth)

    def term_from(self, v: float, depth: int) -> float:
        while (op := self.peek()) in ("*", "/"):
            self.pos += 1
            r = self.factor(depth)
            if op == "/" and r == 0:
                raise _Fail
            v = _finite(v * r if op == "*" else v / r)
        return v

    def factor(self, depth: int) -> float:  # ('+' | '-') factor | number | '(' expr ')'
        if depth >= MAX_DEPTH:
            raise _Fail
        c = self.peek()
        if c is None:
            raise _Fail
        if c == "+":
            self.pos += 1
            return self.factor(depth + 1)
        if c == "-":
            self.pos += 1
            return -self.factor(depth + 1)
        if c == "(":
            self.pos += 1
            v = self.expr(depth + 1)
            if self.peek() != ")":
                raise _Fail
            self.pos += 1
            return v
        return self.number()

    def number(self) -> float:
        start = self.pos
        while (c := self.peek()) is not None and c.isascii() and (c.isdigit() or c == "."):
            self.pos += 1
        try:
            return _finite(float("".join(self.chars[start : self.pos])))
        except ValueError as e:
            raise _Fail from e

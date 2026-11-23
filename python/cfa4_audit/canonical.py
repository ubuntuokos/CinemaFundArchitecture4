"""Canonical form and hash of an audit event (CFA4-AUDIT-001 AU-1, G2-01 section 8).

The same rules as the Rust ``cfa4-audit`` crate, byte for byte; both read the shared vectors in
``tests/vectors/audit/v1``. RFC 8785 (JCS), narrowed on purpose:

* **no floating-point number**: an int in the int64 range only (money and ratios are decimal strings);
* object keys sorted by UTF-16 code units (RFC 8785 3.2.3), not by code point;
* strings escaped minimally: ``"`` and ``\\``, the controls below U+0020 as ``\\b \\t \\n \\f \\r`` or
  ``\\u00xx`` (lower-case hex); everything else, non-ASCII and ``/`` included, stays UTF-8;
* no whitespace.

``hash = SHA-256("cfa4-audit/v1" || 0x00 || canonical bytes)``; :func:`event_hash` covers every field of
the event except ``hash`` itself.
"""

from __future__ import annotations

import hashlib
from typing import Any

from .errors import EventNotObject, FloatForbidden, InvalidValue

HASH_DOMAIN_V1 = b"cfa4-audit/v1"
GENESIS_PREV_HASH = bytes(32)

_INT64_MIN = -(2**63)
_INT64_MAX = 2**63 - 1
_SHORT_ESCAPES = {'"': '\\"', "\\": "\\\\", "\b": "\\b", "\t": "\\t", "\n": "\\n", "\f": "\\f", "\r": "\\r"}


def canonical_bytes(value: Any) -> bytes:
    """The canonical UTF-8 bytes of a JSON value; raises :class:`AuditError` subclasses."""
    out: list[str] = []
    _write(value, out)
    try:
        return "".join(out).encode("utf-8")
    except UnicodeEncodeError as exc:  # a lone surrogate in a string
        raise InvalidValue("string is not valid Unicode") from exc


def event_hash(event: dict[str, Any]) -> bytes:
    """The hash of an event object: every field except ``hash``."""
    if not isinstance(event, dict):
        raise EventNotObject()
    return hash_canonical(canonical_bytes({k: v for k, v in event.items() if k != "hash"}))


def hash_canonical(canonical: bytes) -> bytes:
    """``SHA-256(domain || 0x00 || canonical)`` of already canonical bytes."""
    return hashlib.sha256(HASH_DOMAIN_V1 + b"\x00" + canonical).digest()


def _write(value: Any, out: list[str]) -> None:
    if value is None:
        out.append("null")
    elif value is True:  # bool before int: bool is an int subclass
        out.append("true")
    elif value is False:
        out.append("false")
    elif isinstance(value, int):
        if not _INT64_MIN <= value <= _INT64_MAX:
            raise FloatForbidden("integer outside the int64 range")
        out.append(str(value))
    elif isinstance(value, float):
        raise FloatForbidden("floating-point number")
    elif isinstance(value, str):
        _write_string(value, out)
    elif isinstance(value, (list, tuple)):
        out.append("[")
        for i, item in enumerate(value):
            if i:
                out.append(",")
            _write(item, out)
        out.append("]")
    elif isinstance(value, dict):
        for key in value:
            if not isinstance(key, str):
                raise InvalidValue("object key is not a string")
        out.append("{")
        # UTF-16 big-endian bytes compare exactly like UTF-16 code units. A lone surrogate cannot be
        # encoded and is rejected as invalid.
        try:
            keys = sorted(value, key=lambda k: k.encode("utf-16-be"))
        except UnicodeEncodeError as exc:
            raise InvalidValue("object key is not valid Unicode") from exc
        for i, key in enumerate(keys):
            if i:
                out.append(",")
            _write_string(key, out)
            out.append(":")
            _write(value[key], out)
        out.append("}")
    else:
        raise InvalidValue(f"{type(value).__name__} is not a JSON value")


def _write_string(s: str, out: list[str]) -> None:
    out.append('"')
    for ch in s:
        short = _SHORT_ESCAPES.get(ch)
        if short is not None:
            out.append(short)
        elif ord(ch) < 0x20:
            out.append(f"\\u{ord(ch):04x}")
        else:
            out.append(ch)
    out.append('"')

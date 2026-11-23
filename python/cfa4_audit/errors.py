"""Audit errors: every error has a stable English code; the UI translates it (BacklotHub #16)."""

from __future__ import annotations


class AuditError(Exception):
    """Base of all audit errors. ``code`` is the stable machine-readable code."""

    code = "AUDIT_ERROR"

    def __init__(self, detail: str = "") -> None:
        super().__init__(f"{self.code}: {detail}" if detail else self.code)
        self.detail = detail


class FloatForbidden(AuditError):
    """A number that is not an integer in the int64 range (money and ratios are decimal strings)."""

    code = "AUDIT_FLOAT_FORBIDDEN"


class EventNotObject(AuditError):
    """The event to hash is not a JSON object."""

    code = "AUDIT_EVENT_NOT_OBJECT"


class InvalidValue(AuditError):
    """A value that cannot be canonicalised (not JSON, lone surrogate, non-string key)."""

    code = "AUDIT_INVALID_VALUE"


class InvalidEvent(AuditError):
    """An event field fails validation (shape, format, forbidden content)."""

    code = "AUDIT_EVENT_INVALID"

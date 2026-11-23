"""AU-1: the Python canonical form and hash against the shared vectors (also read by the Rust crate)."""

from __future__ import annotations

import json
import uuid
from datetime import UTC, datetime, timedelta, timezone
from pathlib import Path
from typing import Any

import pytest

from cfa4_audit import (
    GENESIS_PREV_HASH,
    ActorKind,
    AuditError,
    AuditEvent,
    EventNotObject,
    FloatForbidden,
    InvalidEvent,
    InvalidValue,
    Outcome,
    canonical_bytes,
    event_hash,
    format_timestamp,
    new_salt,
    redact,
    uuid7,
)

VECTORS = Path(__file__).resolve().parents[2] / "tests" / "vectors" / "audit" / "v1"


def _vectors() -> list[dict[str, Any]]:
    loaded: list[dict[str, Any]] = [json.loads(p.read_text(encoding="utf-8")) for p in sorted(VECTORS.glob("*.json"))]
    return loaded


def test_the_shared_vectors_are_present() -> None:
    assert len(_vectors()) >= 10


@pytest.mark.parametrize("vec", _vectors(), ids=lambda v: str(v["name"]))
def test_shared_vector(vec: dict[str, Any]) -> None:
    value = json.loads(vec["input"])
    expected_error = vec.get("error")
    if expected_error is not None:
        with pytest.raises(AuditError) as caught:
            event_hash(value) if vec["event"] else canonical_bytes(value)
        assert caught.value.code == expected_error
        return
    if vec["event"]:
        without_hash = {k: v for k, v in value.items() if k != "hash"}
        assert canonical_bytes(without_hash).decode("utf-8") == vec["canonical"]
        assert event_hash(value).hex() == vec["hash"]
    else:
        assert canonical_bytes(value).decode("utf-8") == vec["canonical"]


def test_booleans_are_not_numbers_and_floats_are_rejected() -> None:
    assert canonical_bytes([True, False, 1]) == b"[true,false,1]"
    with pytest.raises(FloatForbidden):
        canonical_bytes({"a": 1.0})
    with pytest.raises(FloatForbidden):
        canonical_bytes(2**63)
    with pytest.raises(FloatForbidden):
        canonical_bytes(-(2**63) - 1)


def test_invalid_values_are_rejected_not_crashed() -> None:
    with pytest.raises(InvalidValue):
        canonical_bytes({1: "a"})
    with pytest.raises(InvalidValue):
        canonical_bytes({"a": object()})
    with pytest.raises(InvalidValue):
        canonical_bytes("\ud800")  # a lone surrogate
    with pytest.raises(InvalidValue):
        canonical_bytes({"\ud800": 1})
    with pytest.raises(EventNotObject):
        event_hash([1])  # type: ignore[arg-type]


def test_error_codes_are_stable() -> None:
    assert FloatForbidden().code == "AUDIT_FLOAT_FORBIDDEN"
    assert EventNotObject().code == "AUDIT_EVENT_NOT_OBJECT"
    assert InvalidEvent("x").code == "AUDIT_EVENT_INVALID"
    assert str(FloatForbidden("detail")) == "AUDIT_FLOAT_FORBIDDEN: detail"


def _event(**changes: Any) -> AuditEvent:
    base: dict[str, Any] = {
        "app": "backlot",
        "action": "production.created",
        "actor_kind": ActorKind.OS_USER,
        "actor_ref": "uid:1000",
        "subject_type": "production",
        "subject_id": uuid.UUID("018f0000-0000-7000-8000-000000000001"),
        "outcome": Outcome.SUCCESS,
        "occurred_at": datetime(2026, 11, 22, 12, 0, 0, 123456, tzinfo=UTC),
    }
    base.update(changes)
    return AuditEvent(**base)


def test_uuid7_has_version_variant_and_rising_time() -> None:
    a, b = uuid7(), uuid7()
    assert a.version == 7 and a.variant == uuid.RFC_4122
    assert a.int >> 80 <= b.int >> 80


def test_timestamps_are_utc_microseconds() -> None:
    plus_two = timezone(timedelta(hours=2))
    assert format_timestamp(datetime(2026, 11, 22, 14, 0, tzinfo=plus_two)) == "2026-11-22T12:00:00.000000Z"
    with pytest.raises(InvalidEvent):
        format_timestamp(datetime(2026, 11, 22))


def test_hashable_form_is_hashed_with_seq_and_prev_hash() -> None:
    ev = _event()
    moment = datetime(2026, 11, 22, 12, 0, 1, tzinfo=UTC)
    first = ev.hashable(seq=1, recorded_at=moment, prev_hash=GENESIS_PREV_HASH)
    assert first["prev_hash"] == "00" * 32 and first["occurred_at"] == "2026-11-22T12:00:00.123456Z"
    assert event_hash(first) != event_hash(ev.hashable(seq=2, recorded_at=moment, prev_hash=GENESIS_PREV_HASH))
    assert event_hash(first) != event_hash(ev.hashable(seq=1, recorded_at=moment, prev_hash=bytes([1]) * 32))
    with pytest.raises(InvalidEvent):
        ev.hashable(seq=0, recorded_at=moment, prev_hash=GENESIS_PREV_HASH)
    with pytest.raises(InvalidEvent):
        ev.hashable(seq=1, recorded_at=moment, prev_hash=b"short")


@pytest.mark.parametrize(
    "changes",
    [
        {"action": "Bad Action"},
        {"action": "nodot"},
        {"reason_code": "lower"},
        {"app": ""},
        {"event_id": uuid.UUID(int=1)},
        {"occurred_at": datetime(2026, 1, 1)},
        {"outcome": Outcome.REJECTED},
        {"schema_ver": 2},
        {"scope": {"amount": 1.5}},
        {"change": {"x": {"old": 0.1, "new": 2}}},
    ],
)
def test_invalid_events_are_refused(changes: dict[str, Any]) -> None:
    with pytest.raises(AuditError):
        _event(**changes)


def test_a_rejected_event_with_reason_is_valid() -> None:
    assert _event(outcome=Outcome.REJECTED, reason_code="PERMISSION_DENIED").reason_code == "PERMISSION_DENIED"


def test_salted_digest_differs_per_salt_and_hides_the_value() -> None:
    a, b = new_salt(), new_salt()
    assert len(a) == 16 and a != b
    one, two = redact("+36 30 123 4567", a), redact("+36 30 123 4567", b)
    assert one["redacted"] is True and len(one["digest"]) == 64
    assert one["digest"] != two["digest"], "same value, different salt: not comparable across rows"
    assert redact("+36 30 123 4567", a) == one, "same salt and value: stable, so verifiable until the salt is erased"
    assert "+36" not in json.dumps(one)
    with pytest.raises(InvalidEvent):
        redact("x", b"short")

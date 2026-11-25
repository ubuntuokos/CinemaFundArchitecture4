"""Render notices: warning and fallback events of ``render.list`` reach the user once."""

from __future__ import annotations

from cfa4_qt.render_notices import new_notices


def _entry(job, state, event, label="Export"):
    return {"job": job, "label": label, "state": state, "event": event}


FALLBACK = {"kind": "fallback", "fallback": {"from": "vapoursynth", "to": "skip", "reason": "worker did not start"}}
WARNING = {"kind": "warning", "message": "out of memory"}


def test_a_fallback_is_announced_once():
    seen = set()
    reply = [_entry(1, "running", FALLBACK)]
    assert new_notices(reply, seen) == [("Export: fell back from vapoursynth to skip (worker did not start)", False)]
    assert new_notices(reply, seen) == []


def test_a_failed_job_warning_is_an_error():
    assert new_notices([_entry(2, "failed", WARNING)], set()) == [("Export: out of memory", True)]
    assert new_notices([_entry(3, "running", WARNING)], set()) == [("Export: out of memory", False)]


def test_progress_and_cancel_say_nothing():
    progress = {"kind": "progress", "progress": {"done": 1, "total": 2}}
    assert new_notices([_entry(4, "running", progress), _entry(5, "cancelled", {"kind": "progress"})], set()) == []


def test_hostile_input_is_skipped_not_raised():
    seen = set()
    assert new_notices(None, seen) == []
    assert new_notices("x", seen) == []
    bad = [None, 3, {}, {"event": 5}, {"event": {"kind": "fallback"}}, {"event": {"kind": "warning", "message": " "}}]
    assert new_notices(bad, seen) == []

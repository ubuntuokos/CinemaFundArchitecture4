"""CFA4-BRANCH-OWNERSHIP-001 T3 (BO-4): the local push guard, without network access."""

from __future__ import annotations

import importlib.util
import sys
from datetime import UTC, datetime
from pathlib import Path
from types import ModuleType
from typing import Any

ROOT = Path(__file__).resolve().parents[2]


def _load(name: str, path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


BO = _load("branch_ownership", ROOT / "scripts" / "ci" / "branch_ownership.py")
GUARD = _load("branch_owner_guard", ROOT / "scripts" / "dev" / "branch_owner_guard.py")

A = "session_01AAAAAAAAAAAAAAAAAAAAAA"
B = "session_01BBBBBBBBBBBBBBBBBBBBBB"
G = "session_01GGGGGGGGGGGGGGGGGGGGGG"
NOW = datetime(2026, 11, 15, 12, tzinfo=UTC)


def _commit(session: str | None) -> Any:
    msg = "BO-4: x\n" + (f"Claude-Session: https://claude.ai/code/{session}\n" if session else "")
    return BO.Commit(sha="a" * 40, message=msg, author="", date=NOW, parents=("p",))


def _pr(body: str) -> dict[str, Any]:
    return {"number": 7, "body": body}


def test_push_target_parsing() -> None:
    assert GUARD.push_target("ls -la") is None
    assert GUARD.push_target("git status && git log") is None
    assert GUARD.push_target("git push") == (None, None, None)
    assert GUARD.push_target("git -C /r -c push.negotiate=true push -q origin HEAD:feat/x") == ("/r", "HEAD", "feat/x")
    assert GUARD.push_target("git push -u origin feat/y 2>&1 | tail -3") == (None, "feat/y", "feat/y")
    assert GUARD.push_target("cd x && git push --force-with-lease origin +main:refs/heads/z") == (None, "main", "z")


def test_owner_may_push() -> None:
    ok, _ = GUARD.decide(_pr(f"Munkamenet: {A}"), [_commit(A)], [], NOW, BO)
    assert ok


def test_foreign_session_is_stopped() -> None:
    ok, msg = GUARD.decide(_pr(f"Munkamenet: {A}"), [_commit(B)], [], NOW, BO)
    assert not ok and "§2.11" in msg


def test_handover_lets_the_new_session_push_and_stops_the_old() -> None:
    body = f"Munkamenet: {A}\nÁtadva: {A} → {B}, tulajdonosi jóváhagyással, 2026-11-15"
    assert GUARD.decide(_pr(body), [_commit(B)], [], NOW, BO)[0]
    assert not GUARD.decide(_pr(body), [_commit(A)], [], NOW, BO)[0]


def test_gazda_after_takeover_and_developer_blocked() -> None:
    comments = [(datetime(2026, 11, 15, 9, tzinfo=UTC), f"Gazda átvette: {G}, 2026-11-15")]
    assert GUARD.decide(_pr(f"Munkamenet: {A}"), [_commit(G)], comments, NOW, BO)[0]
    assert not GUARD.decide(_pr(f"Munkamenet: {A}"), [_commit(A)], comments, NOW, BO)[0]


def test_projects_thread_is_stopped_until_listed() -> None:
    body = f"[project thread](https://claude.ai/code/project/chan_1?thread=c)\n\nMunkamenet: {A}"
    ok, msg = GUARD.decide(_pr(body), [_commit(B)], [], NOW, BO)
    assert not ok and "Munkamenet:" in msg
    listed = body + f" {B}"
    assert GUARD.decide(_pr(listed), [_commit(B)], [], NOW, BO)[0]


def test_no_pr_or_no_owner_allows() -> None:
    assert GUARD.decide(None, [_commit(B)], [], NOW, BO)[0]
    assert GUARD.decide(_pr("no session"), [_commit(B)], [], NOW, BO)[0]

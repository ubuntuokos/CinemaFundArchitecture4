"""CFA4-BRANCH-OWNERSHIP-001 T2 (BO-2): the branch-ownership classifier, without network access.

The cases are the plan's §8 acceptance list."""

from __future__ import annotations

import importlib.util
import sys
from datetime import UTC, datetime
from pathlib import Path
from types import ModuleType
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci" / "branch_ownership.py"

A = "session_01AAAAAAAAAAAAAAAAAAAAAA"
B = "session_01BBBBBBBBBBBBBBBBBBBBBB"
G = "session_01GGGGGGGGGGGGGGGGGGGGGG"
X = "session_01XXXXXXXXXXXXXXXXXXXXXX"


def _mod() -> ModuleType:
    spec = importlib.util.spec_from_file_location("branch_ownership", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module  # dataclasses resolve annotations through sys.modules
    spec.loader.exec_module(module)
    return module


BO = _mod()


def _at(day: int, hour: int = 12) -> datetime:
    return datetime(2026, 11, day, hour, tzinfo=UTC)


def _c(sha: str, session: str | None, when: datetime, parents: int = 1, author: str = "claude") -> Any:
    msg = f"BO-2: change {sha}\n\nCo-Authored-By: Claude\n"
    if session:
        msg += f"Claude-Session: https://claude.ai/code/{session}\n"
    return BO.Commit(sha=sha * 7, message=msg, author=author, date=when, parents=tuple(f"p{sha}{i}" for i in range(parents)))


def _pr(body: str, ref: str = "feature/x", labels: tuple[str, ...] = ()) -> dict[str, Any]:
    return {"number": 1, "head": {"ref": ref, "sha": "h"}, "base": {"ref": "main"}, "body": body, "labels": [{"name": n} for n in labels]}


def _run(
    pr: dict[str, Any],
    commits: list[Any],
    comments: list[tuple[datetime, str]] | None = None,
    inherited: dict[str, int] | None = None,
    merge_ok: bool = True,
) -> Any:
    return BO.classify(pr, commits, comments or [], inherited or {}, lambda _parent: merge_ok)


def test_single_session_is_green() -> None:
    v = _run(_pr(f"Munkamenet: https://claude.ai/code/{A}"), [_c("a", A, _at(14)), _c("b", A, _at(15))])
    assert v.ok and v.owners == [A]


def test_foreign_session_is_red() -> None:
    v = _run(_pr(f"Munkamenet: https://claude.ai/code/{A}"), [_c("a", A, _at(14)), _c("b", X, _at(15))])
    assert [f.sha for f in v.violations] == ["bbbbbbb"]


def test_owner_falls_back_to_first_own_commit() -> None:
    v = _run(_pr("no session here"), [_c("a", A, _at(14)), _c("b", A, _at(15))])
    assert v.ok and v.owners == [A]


def test_owner_falls_back_to_trailing_session_link() -> None:
    v = _run(_pr(f"Summary\n\nhttps://claude.ai/code/{B}"), [_c("a", B, _at(14))])
    assert v.ok and v.owners == [B]


def test_merge_of_main_or_stacked_branch_is_green() -> None:
    v = _run(_pr(f"Munkamenet: {A}"), [_c("a", A, _at(14)), _c("m", X, _at(15), parents=2)], merge_ok=True)
    assert v.ok


def test_merge_of_unrelated_branch_is_red() -> None:
    v = _run(_pr(f"Munkamenet: {A}"), [_c("a", A, _at(14)), _c("m", A, _at(15), parents=2)], merge_ok=False)
    assert len(v.violations) == 1


def test_handover_allows_new_and_blocks_old() -> None:
    body = f"Munkamenet: {A}\nÁtadva: {A} → {B}, tulajdonosi jóváhagyással, 2026-11-15"
    commits = [_c("a", A, _at(14)), _c("b", B, _at(15, 10)), _c("c", A, _at(15, 11))]
    v = _run(_pr(body), commits)
    assert [f.sha for f in v.violations] == ["ccccccc"]


def test_gazda_takeover_and_give_back() -> None:
    body = f"Munkamenet: {A}"
    comments = [
        (_at(15, 9), f"Gazda átvette: {G}, 2026-11-15"),
        (_at(15, 13), f"Gazda visszaadta: {A}, 2026-11-15"),
    ]
    commits = [
        _c("a", A, _at(14)),  # before takeover: developer ok
        _c("g", G, _at(14, 13)),  # gazda before takeover: foreign
        _c("h", G, _at(15, 10)),  # gazda after takeover: ok
        _c("d", A, _at(15, 11)),  # developer after takeover: foreign
        _c("e", A, _at(15, 14)),  # developer after give-back: ok
    ]
    v = _run(_pr(body), commits, comments)
    assert sorted(f.sha for f in v.violations) == ["ddddddd", "ggggggg"]


def test_inherited_commits_are_skipped_and_reported() -> None:
    inherited = {"x" * 7: 1270}
    v = _run(_pr(f"Munkamenet: {B}"), [_c("x", X, _at(14)), _c("b", B, _at(15))], inherited=inherited)
    assert v.ok and v.depends_on == [1270]


def test_projects_thread_sessions_on_the_line_are_owners() -> None:
    body = f"[project thread](https://claude.ai/code/project/chan_1?thread=c)\n\nMunkamenet: {A}, {B}"
    assert _run(_pr(body), [_c("a", A, _at(14)), _c("b", B, _at(15))]).ok
    body_missing = f"[project thread](https://claude.ai/code/project/chan_1?thread=c)\n\nMunkamenet: {B}"
    assert not _run(_pr(body_missing), [_c("a", A, _at(14)), _c("b", B, _at(15))]).ok


def test_owner_commit_without_session_needs_label() -> None:
    commits = [_c("o", None, _at(14), author="ubuntuokos")]
    assert not _run(_pr("x", ref="ubuntuokos-patch-2"), commits).ok
    assert _run(_pr("x", ref="ubuntuokos-patch-2", labels=("tulajdonosi-commit",)), commits).ok


def test_dependabot_branch() -> None:
    ok = [_c("d", None, _at(14), author="dependabot[bot]")]
    assert _run(_pr("", ref="dependabot/cargo/x"), ok).ok
    bad = ok + [_c("e", A, _at(15))]
    assert not _run(_pr("", ref="dependabot/cargo/x"), bad).ok


def test_batch_branch_listed_and_unlisted_heads() -> None:
    body = f"Köteg: #10, #11\n\nMunkamenet: {G}"
    commits = [_c("x", X, _at(14)), _c("m", G, _at(15), parents=2)]
    listed = _run(_pr(body, ref="koteg/20261115-gazda1"), commits, inherited={"x" * 7: 10}, merge_ok=True)
    assert listed.ok
    unlisted = _run(_pr(body, ref="koteg/20261115-gazda1"), commits, inherited={"x" * 7: 10}, merge_ok=False)
    assert not unlisted.ok

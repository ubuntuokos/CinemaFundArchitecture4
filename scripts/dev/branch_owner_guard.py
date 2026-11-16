#!/usr/bin/env python3
"""CFA4-BRANCH-OWNERSHIP-001 T3 (BO-4): stop a `git push` onto a PR branch that belongs to another session.

Runs as a Claude Code `PreToolUse` hook on Bash (`.claude/settings.json`). It reads the hook JSON from stdin,
does nothing unless the command is a `git push`, finds the open PR of the target branch, and checks the
commits about to be pushed: each one's `Claude-Session:` line must name a session that may write to the
branch now (the PR description's `Munkamenet:` line, hand-over and gazda takeover lines; the same rules as
`scripts/ci/branch_ownership.py`). The session id is read from the commits being pushed, because the
environment does not carry it in that form.

Exit 0 = allow; exit 2 = block, with the reason on stderr (Claude Code shows it to the agent). Any error of
its own (no network, no `gh`) allows the push with a warning: the CI check still runs on the PR.
Batch branches (`koteg/*`) are the gazda's and are not checked here. A Claude Projects thread whose new
session is missing from the `Munkamenet:` line is stopped too (D12, plan 1.6): it must add itself to the line first.
"""

from __future__ import annotations

import importlib.util
import json
import re
import shlex
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path
from types import ModuleType
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
OWNERSHIP = ROOT / "scripts" / "ci" / "branch_ownership.py"
PROJECT_THREAD = re.compile(r"https://claude\.ai/code/project/")


def _ownership() -> ModuleType:
    spec = importlib.util.spec_from_file_location("branch_ownership", OWNERSHIP)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def push_target(command: str) -> tuple[str | None, str | None, str | None] | None:
    """(repo dir from `-C`, local source ref, remote branch) of the first `git push` in `command`, or None
    when the command has no `git push`. Source and branch are None for a bare `git push`."""
    for part in re.split(r"&&|\|\||;|\|", command):
        try:
            words = shlex.split(part)
        except ValueError:
            continue
        if "git" not in words:
            continue
        words = words[words.index("git") + 1 :]
        repo: str | None = None
        while words and words[0].startswith("-"):
            opt = words.pop(0)
            if opt == "-C" and words:
                repo = words.pop(0)
            elif opt == "-c" and words:
                words.pop(0)
        if not words or words[0] != "push":
            continue
        args = [w for w in words[1:] if not w.startswith("-")]
        if len(args) < 2:
            return repo, None, None  # current branch to its upstream
        src, _, dst = args[1].partition(":")
        src = src.lstrip("+")
        dst = (dst or src).removeprefix("refs/heads/")
        return repo, src or None, dst or None
    return None


def _git(repo: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, check=True).stdout.strip()


def decide(
    pr: dict[str, Any] | None,
    pushed: list[Any],
    comments: list[tuple[datetime, str]],
    now: datetime,
    bo: ModuleType,
) -> tuple[bool, str]:
    """Pure decision: (allowed, message) for the commits about to be pushed onto `pr`'s branch."""
    if pr is None:
        return True, ""
    body = pr.get("body") or ""
    owners = bo.owners_from_description(body)
    events = bo.events_from_text(body, comments)
    allowed = bo.writers_at(owners, events, now)
    sessions = sorted({c.session for c in pushed if c.session and not c.is_merge})
    foreign = [s for s in sessions if s not in allowed]
    if not foreign or not owners:
        return True, ""
    if PROJECT_THREAD.search(body) and not events:
        return False, (
            f"A push megállítva: a #{pr['number']} egy Claude Projects-szál PR-ja, és a munkamenet "
            f"({', '.join(foreign)}) nincs a leírás `Munkamenet:` sorában. Fűzd hozzá a sorhoz, majd told újra "
            "(docs/coordination.md §2.4, D12)."
        )
    return False, (
        f"A push megállítva: a #{pr['number']} ága most ezeké: {', '.join(sorted(allowed))}; a pusholt commitok "
        f"munkamenete: {', '.join(foreign)}. Más PR-jához ne nyúlj (docs/coordination.md §2.4). Ha a tulajdonos "
        "rád bízta, előbb írd be az `Átadva:` sort (§2.11); gazdaként a `Gazda átvette:` sort (§2.12); "
        "különben nyiss új ágat a main-ből."
    )


def main() -> int:
    try:
        event = json.load(sys.stdin)
        command = str(event.get("tool_input", {}).get("command", ""))
        target = push_target(command)
        if target is None:
            return 0
        repo_arg, src, dst = target
        repo = Path(event.get("cwd") or ".")
        if repo_arg:
            repo = (repo / repo_arg).resolve()
        if dst is None:
            dst = _git(repo, "branch", "--show-current")
            src = "HEAD"
        if not dst or dst.startswith("koteg/"):
            return 0
        bo = _ownership()
        owner = bo.REPO.split("/")[0]
        prs = bo._get(f"pulls?state=open&head={owner}:{dst}&per_page=5", paginate=False)
        pr = prs[0] if prs else None
        if pr is None:
            return 0
        shas = _git(repo, "rev-list", src or "HEAD", "--not", "--remotes").split()
        pushed = []
        for sha in shas:
            msg = _git(repo, "log", "-1", "--format=%B", sha)
            parents = _git(repo, "log", "-1", "--format=%P", sha).split()
            pushed.append(bo.Commit(sha=sha, message=msg, author="", date=datetime.now(UTC), parents=tuple(parents)))
        comments = [
            (bo.parse_date(c["created_at"]), c.get("body") or "")
            for c in bo._get(f"issues/{pr['number']}/comments?per_page=100")
        ]
        ok, message = decide(pr, pushed, comments, datetime.now(UTC), bo)
    except Exception as exc:  # noqa: BLE001 - the guard must never break a push by itself
        print(f"branch-owner-guard: nem tudtam ellenőrizni ({exc}); a push mehet, a CI-check ellenőriz.", file=sys.stderr)
        return 0
    if message:
        print(message, file=sys.stderr)
    return 0 if ok else 2


if __name__ == "__main__":
    sys.exit(main())

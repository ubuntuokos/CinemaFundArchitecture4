#!/usr/bin/env python3
"""CFA4-BRANCH-OWNERSHIP-001 T2: which session may write to a PR branch, and did anyone else.

One PR branch belongs to one owner: the sessions named on the PR description's `Munkamenet:` line (D9), or,
without that line, the session of the PR's first own commit. Hand-overs (`Átadva:`), gazda takeover
(`Gazda átvette:` / `Gazda visszaadta:`), stacked PRs (D10), commits inherited from another open PR (D11),
Claude Projects threads (D12), Dependabot and batch (`koteg/*`) branches are handled as the plan says.

Usage:  branch_ownership.py --pr N [--mode report|enforce]
        branch_ownership.py --all-open                      (read-only overview, BO-1 style)

Reads the GitHub API with GITHUB_TOKEN (CI) or through the `gh` CLI (local). Never writes to GitHub.
In `report` mode (the plan's one-week D4 period) a violation is a warning and the exit code is 0;
in `enforce` mode it is an error and the exit code is 1.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.request
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Any

REPO = os.environ.get("GITHUB_REPOSITORY", "ubuntuokos/CinemaFundArchitecture4")
OWNER_LABEL = "tulajdonosi-commit"

SESSION = r"session_[A-Za-z0-9]+"
COMMIT_SESSION = re.compile(rf"^Claude-Session:\s*https://claude\.ai/code/({SESSION})\s*$", re.M)
DESC_OWNER = re.compile(r"^\s*(?:\*\*)?Munkamenet:?(?:\*\*)?:?(.*)$", re.M)
SESSION_ID = re.compile(SESSION)
HANDOVER = re.compile(rf"Átadva:\s*({SESSION})\s*(?:→|->)\s*({SESSION}).*?(\d{{4}}-\d{{2}}-\d{{2}})")
GAZDA_TAKES = re.compile(rf"Gazda átvette:\s*({SESSION}).*?(\d{{4}}-\d{{2}}-\d{{2}})")
GAZDA_GIVES = re.compile(rf"Gazda visszaadta:\s*({SESSION}).*?(\d{{4}}-\d{{2}}-\d{{2}})")
LISTED_PR = re.compile(r"#(\d+)")

Json = dict[str, Any]


@dataclass(frozen=True)
class Commit:
    sha: str
    message: str
    author: str
    date: datetime
    parents: tuple[str, ...]

    @property
    def session(self) -> str | None:
        m = COMMIT_SESSION.search(self.message)
        return m.group(1) if m else None

    @property
    def is_merge(self) -> bool:
        return len(self.parents) > 1


@dataclass(frozen=True)
class Event:
    """A change of who may write: hand-over, gazda takeover or give-back, at a point in time."""

    when: datetime
    kind: str  # "handover" | "gazda_takes" | "gazda_gives"
    old: str | None
    new: str


COMMIT_LIST_LIMIT = 250  # GitHub's pulls/{n}/commits endpoint never returns more


@dataclass
class Finding:
    sha: str
    reason: str


@dataclass
class Verdict:
    number: int
    kind: str
    owners: list[str]
    violations: list[Finding] = field(default_factory=list)
    depends_on: list[int] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.violations


def parse_date(text: str) -> datetime:
    return datetime.fromisoformat(text.replace("Z", "+00:00"))


def owners_from_description(body: str) -> list[str]:
    """D9/D12: every session id on the `Munkamenet:` line(s) of the description, in order. Until every PR
    has that line, a description without one falls back to the session links it carries anywhere (the
    trailing `https://claude.ai/code/session_…` line every Claude PR ends with), hand-over lines excluded."""
    found: list[str] = []
    for m in DESC_OWNER.finditer(body):
        for s in SESSION_ID.findall(m.group(1)):
            if s not in found:
                found.append(s)
    if found:
        return found
    for line in body.splitlines():
        if HANDOVER.search(line) or GAZDA_TAKES.search(line) or GAZDA_GIVES.search(line):
            continue
        for s in re.findall(rf"https://claude\.ai/code/({SESSION})", line):
            if s not in found:
                found.append(s)
    return found


def events_from_text(body: str, comments: Iterable[tuple[datetime, str]]) -> list[Event]:
    """Hand-over and takeover lines. A comment carrying the same line gives the exact time; otherwise the
    line's date counts from the start of that day (UTC)."""
    timed = list(comments)

    def when(line: str, day: str) -> datetime | None:
        for at, text in timed:
            if line in text:
                return at
        try:
            return datetime.fromisoformat(day).replace(tzinfo=UTC)
        except ValueError:
            print(f"figyelem: érvénytelen dátum, az esemény kimarad: {line.strip()}", file=sys.stderr)
            return None

    events: list[Event] = []

    def add(kind: str, line: str, day: str, old: str | None, new: str) -> None:
        at = when(line, day)
        if at is not None:
            events.append(Event(at, kind, old, new))

    for text in [body, *(t for _, t in timed)]:
        for m in HANDOVER.finditer(text):
            add("handover", m.group(0), m.group(3), m.group(1), m.group(2))
        for m in GAZDA_TAKES.finditer(text):
            add("gazda_takes", m.group(0), m.group(2), None, m.group(1))
        for m in GAZDA_GIVES.finditer(text):
            add("gazda_gives", m.group(0), m.group(2), None, m.group(1))
    # The same line in the description and in a comment resolves to the same time, so it collapses; separate
    # events with equal sessions at different times stay apart.
    unique = {(e.kind, e.old, e.new, e.when): e for e in events}
    return sorted(unique.values(), key=lambda e: e.when)


def writers_at(owners: list[str], events: list[Event], at: datetime) -> set[str]:
    """The sessions that may write non-merge commits at time `at`."""
    devs = set(owners)
    gazda: str | None = None
    for e in events:
        if e.when > at:
            break
        if e.kind == "handover":
            devs.discard(e.old or "")
            devs.add(e.new)
        elif e.kind == "gazda_takes":
            gazda = e.new
        elif e.kind == "gazda_gives":
            gazda = None
            devs.add(e.new)
    return {gazda} if gazda else devs


def classify(
    pr: Json,
    commits: list[Commit],
    comments: list[tuple[datetime, str]],
    inherited: dict[str, int],
    merge_parent_ok: Callable[[str], bool],
) -> Verdict:
    """Pure decision for one PR. `inherited` maps a commit sha to the open PR it comes from (D11);
    `merge_parent_ok` says whether a merge's second parent is on main, the base branch or another open PR
    head (D10), or, on a batch branch, on main or a listed PR head (D3)."""
    ref: str = pr["head"]["ref"]
    body: str = pr.get("body") or ""
    labels = {lab["name"] for lab in pr.get("labels", [])}
    kind = "dependabot" if ref.startswith("dependabot/") else "köteg" if ref.startswith("koteg/") else "fejlesztői"

    owners = owners_from_description(body)
    own = [c for c in commits if not c.is_merge and c.sha not in inherited]
    if not owners:
        first = next((c.session for c in own if c.session), None)
        if first:
            owners = [first]
    verdict = Verdict(number=pr["number"], kind=kind, owners=owners)
    verdict.depends_on = sorted(set(inherited.values()))
    if not owners and kind == "fejlesztői" and any(c.session for c in own):
        verdict.notes.append("a leírásban nincs `Munkamenet:` sor")
    events = events_from_text(body, comments)
    if len(commits) >= COMMIT_LIST_LIMIT:
        # The endpoint returns at most 250 commits; a later foreign commit would go unseen.
        verdict.violations.append(
            Finding(
                "-",
                f"a PR {COMMIT_LIST_LIMIT} vagy több commitot tartalmaz, a lista hiányos lehet; kézi ellenőrzés kell",
            )
        )

    for c in commits:
        short = c.sha[:7]
        if c.is_merge:
            if not merge_parent_ok(c.parents[1]):
                verdict.violations.append(
                    Finding(
                        short,
                        "merge commit, amelynek második szülője nem a main-en, a báziságon vagy egy másik nyitott PR fején van",
                    )
                )
            continue
        if c.sha in inherited:
            continue
        if kind == "dependabot":
            if not c.author.startswith("dependabot"):
                verdict.violations.append(Finding(short, f"nem Dependabot-commit a Dependabot-ágon ({c.author})"))
            continue
        session = c.session
        if session is None:
            if OWNER_LABEL not in labels:
                verdict.violations.append(
                    Finding(
                        short,
                        f"`Claude-Session:` sor nélküli commit ({c.author}); tulajdonosi commitnál `{OWNER_LABEL}` címke kell (D2)",
                    )
                )
            continue
        allowed = writers_at(owners, events, c.date)
        if session not in allowed:
            verdict.violations.append(
                Finding(short, f"idegen munkamenet: {session}; ekkor írhatott: {', '.join(sorted(allowed)) or '–'}")
            )
    return verdict


# --- GitHub access (read-only) ---------------------------------------------------------------------------


def _get(path: str, paginate: bool = True) -> Any:
    """GET a repository endpoint. List endpoints are followed across pages; others are read once."""
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        cmd = ["gh", "api", *(["--paginate"] if paginate else []), f"repos/{REPO}/{path}"]
        out = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return json.loads(out.stdout.replace("]\n[", ",").replace("][", ",") if paginate else out.stdout)
    url: str | None = f"https://api.github.com/repos/{REPO}/{path}"
    result: Any = None
    while url:
        req = urllib.request.Request(
            url, headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"}
        )
        with urllib.request.urlopen(req, timeout=60) as resp:  # noqa: S310 (fixed https host)
            page = json.load(resp)
            link = resp.headers.get("Link", "")
        result = page if result is None else (result + page if isinstance(page, list) else page)
        nxt = re.search(r'<([^>]+)>;\s*rel="next"', link)
        url = nxt.group(1) if nxt and paginate else None
    return result


def _commits(number: int) -> list[Commit]:
    return [
        Commit(
            sha=c["sha"],
            message=c["commit"]["message"],
            author=(c.get("author") or {}).get("login") or c["commit"]["author"]["name"],
            date=parse_date(c["commit"]["committer"]["date"]),
            parents=tuple(p["sha"] for p in c["parents"]),
        )
        for c in _get(f"pulls/{number}/commits?per_page=100")
    ]


class Repo:
    def __init__(self) -> None:
        self.open_prs: list[Json] = _get("pulls?state=open&per_page=100")
        self._commits: dict[int, list[Commit]] = {}
        self._reach: dict[tuple[str, str], bool] = {}

    def commits(self, number: int) -> list[Commit]:
        if number not in self._commits:
            self._commits[number] = _commits(number)
        return self._commits[number]

    def reachable(self, sha: str, ref: str) -> bool:
        key = (sha, ref)
        if key not in self._reach:
            try:
                status = _get(f"compare/{sha}...{ref}?per_page=1", paginate=False).get("status")
            except (subprocess.CalledProcessError, OSError, ValueError):
                status = None
            self._reach[key] = status in ("ahead", "identical")
        return self._reach[key]

    def check(self, pr: Json) -> Verdict:
        number = pr["number"]
        commits = self.commits(number)
        shas = {c.sha for c in commits}
        others = [p for p in self.open_prs if p["number"] != number]
        batch = pr["head"]["ref"].startswith("koteg/")
        listed = {int(n) for n in LISTED_PR.findall(pr.get("body") or "")} if batch else set()
        # D11: a commit this PR shares with an older open PR (this branch is built on that one) belongs to
        # that PR. A newer PR that shares commits is stacked on this one and inherits from it, not the
        # other way round. On a batch branch the listed PRs are the source of every shared commit.
        created = parse_date(pr["created_at"])
        inherited: dict[str, int] = {}
        for other in sorted(others, key=lambda p: p["created_at"]):
            if batch and other["number"] not in listed:
                continue
            if not batch and parse_date(other["created_at"]) >= created:
                continue
            for c in self.commits(other["number"]):
                if c.sha in shas:
                    inherited.setdefault(c.sha, other["number"])

        def merge_parent_ok(parent: str) -> bool:
            if self.reachable(parent, "main") or self.reachable(parent, pr["base"]["ref"]):
                return True
            heads = [p for p in others if not batch or p["number"] in listed]
            return any(
                parent == p["head"]["sha"] or parent in {c.sha for c in self.commits(p["number"])} for p in heads
            )

        comments = [
            (parse_date(c["created_at"]), c.get("body") or "") for c in _get(f"issues/{number}/comments?per_page=100")
        ]
        return classify(pr, commits, comments, inherited, merge_parent_ok)


def _report(v: Verdict, mode: str) -> str:
    level = "error" if mode == "enforce" else "warning"
    lines = [f"PR #{v.number} ({v.kind}); tulajdonos: {', '.join(v.owners) or '–'}"]
    if v.depends_on:
        lines.append(f"függőség: csak a {', '.join(f'#{n}' for n in v.depends_on)} után olvasztható be (D11)")
    lines += [f"megjegyzés: {n}" for n in v.notes]
    if v.ok:
        lines.append("rendben: nincs idegen commit")
    for f in v.violations:
        lines.append(f"::{level}::{f.sha}: {f.reason}")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else None)
    ap.add_argument("--pr", type=int)
    ap.add_argument("--all-open", action="store_true")
    ap.add_argument("--mode", choices=("report", "enforce"), default="report")
    args = ap.parse_args(argv)
    repo = Repo()
    prs = repo.open_prs if args.all_open else [p for p in repo.open_prs if p["number"] == args.pr]
    if not prs and args.pr:
        prs = [_get(f"pulls/{args.pr}")]
    verdicts = [repo.check(p) for p in sorted(prs, key=lambda p: p["number"])]
    for v in verdicts:
        print(_report(v, args.mode))
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            for v in verdicts:
                fh.write(f"### #{v.number}: {'rendben' if v.ok else f'{len(v.violations)} idegen commit'}\n\n")
                fh.writelines(f"- `{f.sha}`: {f.reason}\n" for f in v.violations)
    bad = any(not v.ok for v in verdicts)
    return 1 if bad and args.mode == "enforce" else 0


if __name__ == "__main__":
    sys.exit(main())

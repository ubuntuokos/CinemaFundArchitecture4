#!/usr/bin/env python3
"""BO-1: read-only branch-ownership report over all open PRs (CFA4-BRANCH-OWNERSHIP-001 §4 T5)."""
import json
import re
import subprocess
import sys

R = "repos/ubuntuokos/cinemafundarchitecture4"
SESS = re.compile(r"Claude-Session:\s*https://claude\.ai/code/(session_[A-Za-z0-9]+)")
DESC_SESS = re.compile(r"https://claude\.ai/code/(session_[A-Za-z0-9]+)")
_main_cache: dict[str, bool] = {}


def api(path: str):
    out = subprocess.run(["gh", "api", "--paginate", path], capture_output=True, text=True, check=True).stdout
    # --paginate concatenates JSON arrays as "][": normalise
    out = out.replace("][", ",")
    return json.loads(out)


def on_main(sha: str) -> bool:
    if sha not in _main_cache:
        c = json.loads(subprocess.run(["gh", "api", f"{R}/compare/{sha}...main"], capture_output=True, text=True).stdout or "{}")
        _main_cache[sha] = c.get("status") in ("ahead", "identical")
    return _main_cache[sha]


def main() -> None:
    prs = api(f"{R}/pulls?state=open&per_page=100")
    rows = []
    for pr in sorted(prs, key=lambda p: p["number"]):
        n, ref, body = pr["number"], pr["head"]["ref"], pr.get("body") or ""
        commits = api(f"{R}/pulls/{n}/commits?per_page=100")
        kind = "dependabot" if ref.startswith("dependabot/") else "köteg" if ref.startswith("koteg/") else "fejlesztői"
        owner = None
        foreign, nosess, badmerge = [], [], []
        for c in commits:
            msg, sha = c["commit"]["message"], c["sha"]
            author = (c.get("author") or {}).get("login") or c["commit"]["author"]["name"]
            parents = c["parents"]
            m = SESS.search(msg)
            if len(parents) > 1:
                if not on_main(parents[1]["sha"]):
                    badmerge.append(sha[:7])
                continue
            if m is None:
                if not author.startswith("dependabot"):
                    nosess.append(f"{sha[:7]} ({author})")
                continue
            s = m.group(1)
            if owner is None:
                owner = s
            elif s != owner:
                foreign.append(f"{sha[:7]} ({s[-8:]})")
        desc = DESC_SESS.findall(body)
        rows.append({
            "n": n, "kind": kind, "ref": ref, "title": pr["title"], "commits": len(commits),
            "owner": owner, "desc_sess": desc[0] if desc else None,
            "desc_matches": bool(owner and owner in desc),
            "foreign": foreign, "nosess": nosess, "badmerge": badmerge,
        })
    json.dump(rows, sys.stdout, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()

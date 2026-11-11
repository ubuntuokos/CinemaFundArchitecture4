#!/usr/bin/env bash
# CFA4 Spec Kit policy check (CFA4-DEV-SPEC-KIT-001, SK-F4 / AC-SK-07).
# Run from the root of a tree where Spec Kit is installed with the cfa4-policy preset
# (throwaway worktree or pilot branch). Exits 1 and lists every violation found.
# Read-only: it never changes a file.
set -u

fail=0
violation() { echo "VIOLATION: $*"; fail=1; }

[ -d .specify ] || { echo "ERROR: no .specify/ here - run from the Spec Kit tree root"; exit 2; }

# 1. Existing CFA4 Claude files must be unchanged (Spec Kit must not write into them).
while IFS= read -r f; do
  case "$f" in .claude/skills/speckit-*) continue ;; esac
  git diff --quiet HEAD -- "$f" 2>/dev/null || violation "tracked Claude file modified: $f"
done < <(git ls-files .claude)

# 2. No Claude event hooks from Spec Kit.
if [ -f .claude/settings.json ] && grep -qiE 'speckit|specify|"hooks"' .claude/settings.json; then
  violation ".claude/settings.json contains Spec Kit hooks"
fi

# 3. No extensions (git, GitHub issues or any other).
[ -e .specify/extensions.yml ] && violation ".specify/extensions.yml exists (extensions/hooks installed)"
if [ -d .specify/extensions ] && [ -n "$(find .specify/extensions -mindepth 1 -maxdepth 1 -type d 2>/dev/null)" ]; then
  violation "extension installed under .specify/extensions/"
fi

# 4. Every Spec Kit core skill is user-invoked only.
n=0
for f in .claude/skills/speckit-*/SKILL.md; do
  [ -e "$f" ] || continue
  n=$((n + 1))
  awk '/^---/{c++; next} c==1 && /^disable-model-invocation: true$/{ok=1} END{exit !ok}' "$f" \
    || violation "model-invocable skill: $f"
done
[ "$n" -eq 0 ] && violation "no .claude/skills/speckit-* skills found"

# 5. Forbidden commands are blocking stubs.
for c in implement taskstoissues constitution; do
  f=".claude/skills/speckit-$c/SKILL.md"
  [ -e "$f" ] || continue
  grep -q "CFA4-BLOCKED" "$f" || violation "speckit.$c is not blocked: $f"
done

# 6. The bundled workflow cannot reach implement (D9 overlay).
if command -v specify >/dev/null 2>&1; then
  out=$(specify workflow resolve speckit 2>&1) || violation "specify workflow resolve speckit failed"
  printf '%s\n' "$out" | grep -qE '^\s*•\s*implement:' && violation "workflow speckit still contains implement"
else
  grep -rqs 'remove: implement' .specify/workflows/overlays 2>/dev/null \
    || violation "D9 overlay (remove: implement) not found and specify CLI unavailable"
fi

# 7. Reference constitution, not a template or generated copy.
c=.specify/memory/constitution.md
if [ ! -f "$c" ]; then
  violation "missing $c"
else
  grep -q "hivatkozó fájl" "$c" || violation "$c is not the CFA4 reference constitution"
  grep -q '\[PROJECT_NAME\]\|<SHA>' "$c" && violation "$c still has template placeholders"
fi

# 8. Spec Kit did not create branches or commits on its own: HEAD must be the expected commit.
if [ -n "${CFA4_EXPECTED_HEAD:-}" ] && [ "$(git rev-parse HEAD)" != "$CFA4_EXPECTED_HEAD" ]; then
  violation "HEAD moved: $(git rev-parse --short HEAD) (expected ${CFA4_EXPECTED_HEAD:0:7})"
fi

if [ "$fail" -eq 0 ]; then echo "cfa4-policy: OK"; fi
exit "$fail"

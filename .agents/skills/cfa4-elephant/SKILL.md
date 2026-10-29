---
name: cfa4-elephant
description: Read authoritative CFA4 project context before optional Elephant-Goldfish design or review work. No autonomous implementation or merge.
---

# CFA4 project-context reader

Use this as an **optional** project-specific companion to the upstream Elephant-Goldfish Model skills, not as a substitute for upstream skills, reviewer personas, or an actual fresh-context review.

## Read current repository state, in order

1. `VISION.md` – governing product goals and priorities.
2. `AGENTS.md` – mandatory architecture, platform, security, licensing, no-crash and testing rules.
3. `docs/coordination.md` and `docs/coordination-log.md` – active PR/merge coordinator discipline.
4. `CLAUDE.md` (if relevant) and `docs/agents.md` – agent working instructions; never override `AGENTS.md`.
5. `docs/architecture.md`, `docs/licensing.md`, `docs/testing.md`, `docs/development/linux.md`.
6. The relevant ADR, design, issue, current source files, tests and open PRs **for the requested topic only**.

## Review invariants

- Preserve Ubuntu 26.04 Linux AMD64 support, Rust media engine and incremental Qt6 GUI.
- Check actual branch/source state; do not assert implemented functionality from plans.
- Owner authorizes tradeoffs and implementation; only the designated coordinator merges. Use fresh `main`, one topic per PR.
- Distinguish **VERIFIED**, **PENDING**, **FAIL** and **SKIPPED** with concrete evidence.
- For a genuine Goldfish review, fresh-context subagents must independently read the design and referenced files. If not available, explicitly record `GOLDFISH_UNAVAILABLE` and stop the Goldfish stage.
- Do not auto-edit source or docs as a side effect of drift detection.
- Do not fetch secrets, ignored `plan/` files, user media or unrelated project context.

For the complete optional workflow, read [docs/development/egm-design-review.md](../../../docs/development/egm-design-review.md).

# CFA4 – EGM design and review integration

Status: **opt-in development workflow (documentation/configuration only)**. No EGM code is vendored; no build, GUI, codec, release or CI gate is changed by this PR.

Source: https://github.com/google/elephant-goldfish-model (Apache-2.0). EGM is a collection of agent skills, not a language model or an executable media backend. Upstream is not officially supported as a Google product.

## Scope and authority

- CFA4 source of truth: `VISION.md`, `AGENTS.md`, `docs/coordination.md`, `docs/coordination-log.md`, `docs/licensing.md` and the relevant design/ADR/test files **in the current checked-out revision**. Do not substitute memory for repository reads.
- This process is **advisory**: it cannot waive owner approval, approve architectural tradeoffs, merge PRs, or replace actual builds, hardware tests, codec oracles, Qt6 parity checks, or the full validation gate.
- Every new topic uses a fresh `main` branch and a dedicated PR. Never touch another session's branch/PR. Only the designated merge coordinator merges after owner approval and green full CI.
- Do not automatically upload sensitive local files, secrets, user footage, host configuration or ignored `plan/` data to a cloud agent.
- No new product dependency: do not install EGM into the shipped application, `pyproject.toml`, `Cargo.toml`, runtime images, `packaging/` or automatic CI.

## Optional setup

Install the **whole** upstream EGM plugin in a compatible development agent. Copying only its `skills/` directory breaks the reviewer persona and standards references. Follow the current upstream [harness guide](https://github.com/google/elephant-goldfish-model/blob/main/docs/harnesses.md). For example, with the user's already configured Codex installation:

```text
codex plugin marketplace add google/elephant-goldfish-model
codex plugin add egm@egm
```

These are **manual opt-in instructions**, not commands executed by this PR. Confirm upstream plugin trust, licence and revision before installing; for reproducibility, record the selected upstream commit SHA in the relevant design review record. A ChatGPT Work session must expose a native fresh-context subagent tool before an EGM Goldfish review is claimed; merely having ChatGPT/Work installed is insufficient.

Use `.agents/skills/cfa4-elephant/SKILL.md` as the CFA4-specific project reading guide. It does not replace upstream EGM skills or inject instructions automatically into every agent.

## Review procedure (per feature)

1. **Intake**: name one feature and one target design document; check the latest `main` and the current owner decisions. First inspect existing designs, open PRs and relevant source to avoid duplicate or conflicting work.
2. **Explore**: run EGM brainstorm when decisions are genuinely open, writing a concept brief. Skip ideation for a small, unambiguous bug fix.
3. **Specify**: use EGM draft to produce a self-contained design containing purpose, scope, acceptance criteria, source paths, architecture, alternatives, licence/provenance, CPU fallback, compatibility, tests and explicit constraints. Mark unsupported assertions as **PENDING**.
4. **Independent review**: run EGM review in real isolated *fresh* subagent context, with access to the draft and explicitly cited source files, but no prior chat history. Never describe a same-context reread as a Goldfish review. If no fresh-context tool exists, record `GOLDFISH_UNAVAILABLE` and run only a plainly labelled ordinary/manual review.
5. **Resolve**: record each finding with a traceable identifier, severity, evidence, decision and disposition; ask the owner to decide architectural tradeoffs. Update the design and re-review major changes.
6. **Approve**: do not start implementation until the owner approves the design when approval is required. EGM output alone is not approval.
7. **Implement and verify**: follow repository tests and PR discipline. Preserve the Rust engine/Qt6 architecture, Ubuntu 26.04 AMD64 release scope, no-crash policy, copyright/licensing and software coexistence.
8. **Detect drift**: run EGM drift-report against the checked-out current designs and source. Report discrepancies with precise paths and evidence; do not silently edit source or documents. A drift report is evidence for review, **not** a PASS assertion or CI replacement.

## Suggested verification record

Store a small report in the feature PR or a dedicated design file:

| Field | Required value |
| --- | --- |
| CFA4 base | `main` commit SHA inspected |
| EGM source | Upstream commit SHA and licence |
| Feature | Target design path, issue/PR |
| Reviewer isolation | Agent/tool/session evidence, or `GOLDFISH_UNAVAILABLE` |
| Findings | Finding ID, evidence path/line, severity, resolution |
| Owner decision | Approved / rejected / pending, with reference |
| Tests | Exact commands, machine/OS, PASS / FAIL / SKIPPED with reason |
| Release gate | Full CI result for **the PR head**, or `NOT_RUN` |

Do not report a skipped test as passed. In particular, FFmpeg/HEIF/oracle, hardware and Qt6 end-to-end coverage must be tested on an appropriate environment before any release-readiness assertion. EGM drift findings do not themselves change the CFA4 release decision.

## Verification criteria for this integration

- These two new text files are present and links resolve.
- The upstream EGM documentation confirms the full-plugin, persona and isolated subagent requirements.
- No runtime dependency, existing CI workflow, product code or owner approval rules were modified.
- Manual installation, independent subagent execution, upstream revision pinning and an end-to-end review are **NOT VERIFIED** until performed in a development environment and captured in a review record.

This integration is intentionally minimal and opt-in: it supplies usable project context and a repeatable review protocol, without asserting unexecuted functionality.

---
title: Planning vs Durable Docs
slug: planning-vs-durable-docs
topic: repo-discipline
summary: "Scattered notes, ad-hoc `TODO.md`, `NOTES.md`, `ROADMAP.md`, `PLAN-foo.md` files, parallel planning docs, and inline `// TODO:` annotations used as a substitute"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:019f12f9-8a0b-7012-ad2f-f4d0cb035d2b
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
  - session:019f12ce-2569-72e0-b959-6d87d5daec5d
---

# Planning vs Durable Docs

## Plans Are Not Durable Documentation

Scattered notes, ad-hoc `TODO.md`, `NOTES.md`, `ROADMAP.md`, `PLAN-foo.md` files, parallel planning docs, and inline `// TODO:` annotations used as a substitute for tracking are forbidden.

Plans are not durable understanding and must not survive as reference documentation after they have been implemented, executed, or invalidated. When a plan completes, it is removed or collapsed to the smallest live follow-up. Lasting knowledge learned from completed work belongs in durable documentation: `docs/product-spec/` for product doctrine, `docs/fabric-architecture*.md` and design docs under `docs/` for architecture, and `docs/wiki/` for source-backed synthesis. An implemented plan is no longer a source of truth; the issue is closed or the temporal detail is deleted, and durable lessons are preserved in the doc that owns that concept.

No new top-level planning files (`PLAN.md`, `TODO.md`, `ROADMAP.md`, `NEXT.md`, `STATUS.md`) are created at the repo root or directly under `docs/`. State is not duplicated across files; a violation or feature tracked in GitHub Issues is not also restated in a separate plan file, maintaining a single source of truth per fact. When a doc is wrong, the authoritative document is edited directly; no appendix or revision section is added to contradict the original while leaving the wrong text intact.

When in doubt, fewer planning files are preferred; if a new planning file feels necessary, the contributor must justify why it cannot be a GitHub issue plus a durable doc. The `Plans/` directory is empty and `M1.md` has been drained into `docs/daemon-design.md`, `docs/fabric-architecture.md`, and `docs/product-spec/`; no new files are added under `Plans/` or new top-level plan docs, as the canonical surfaces are GitHub Issues plus durable docs.

<!-- citations: [^019f1-83994] [^019f1-1e264] [^019f1-bd89b] -->
## TODO Comments

A `// TODO:` comment is not a plan. If it represents work to be done it belongs in a GitHub issue, and if it represents a known limitation or durable decision it belongs in the architecture/design doc or wiki article that owns the concept.

<!-- citations: [^019f1-eb16d] [^019f1-e8d4e] -->
## Review Dumps and Post-Merge Notes

AI/codex review dumps and post-merge review notes must not be committed. Actionable findings are promoted into a GitHub issue or a durable doc, then the review is discarded.

<!-- citations: [^019f1-11045] [^019f1-81dae] -->
# Planning vs Durable Docs

## Plans Are Not Durable Documentation

Plans are not durable understanding and must not survive as reference documentation after they have been implemented, executed, or invalidated. When a plan completes, it is removed or collapsed to the smallest live follow-up; lasting knowledge learned from the work belongs in durable documentation instead. An implemented plan is no longer a source of truth; the issue is closed or the temporal detail is deleted, and durable lessons are preserved in the doc that owns that concept.

## TODO Comments

A `// TODO:` comment is not a plan. If it represents work to be done it belongs in a GitHub issue, and if it represents a known limitation or durable decision it belongs in the architecture/design doc or wiki article that owns the concept.

## Review Dumps and Post-Merge Notes

AI/codex review dumps and post-merge review notes must not be committed. Actionable findings are promoted into a GitHub issue or a durable doc, then the review is discarded.

## Docs and Wiki Artifacts Are Always Committed

Docs, wiki, and other documentation artifacts are always included in commits — never excluded, skipped, or treated as optional. Documentation changes ship with the code they describe. <!-- [^bd868-58e6d] -->

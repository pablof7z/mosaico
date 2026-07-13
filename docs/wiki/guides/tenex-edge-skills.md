---
title: Tenex-Edge Skills
slug: tenex-edge-skills
topic: agent-skills
summary: This guide governs the family of `tenex-edge` agent skills written to `./skills/tenex-edge/` with symlinks from `~/.agents/skills/tenex-edge` and `~/.claude/ski
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-07-13
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:019f12f9-8a0b-7012-ad2f-f4d0cb035d2b
---

# Tenex-Edge Skills

## Overview

This guide governs the `tenex-edge` agent skill written to
`./skills/tenex-edge/`, with user-level symlinks pointing to the repo-local
skill directory. Agent self-identity and full awareness live under `my session`;
human `who` is an operator-only terminal view. Targeting uses public session
handles such as `@codex-quill-peak-369`.

<!-- citations: [^019f1-7100b] [^019f1-106a9] -->
## Skill Set

The `tenex-edge` skill teaches day-to-day agent fabric usage: `my session`,
channel read/send/wait, dispatch, membership, and channel navigation. It also
carries the rule to verify live help before trusting docs.

The `tenex-edge-dev` skill teaches hook wiring and debugging for Codex, Claude
Code, and OpenCode, including how to prove agent registration with `my session`.

The `tenex-edge-verification` skill teaches local gates and test tiers: `just fmt-check`, `just loc-check`, `just lint`, `just test-unit`, when to use `cargo test`, when relay/croissant/nak are required, and how to run e2e safely. Because command and docs drift is already visible, this skill includes small scripts/resources such as a command that prints current help, LOC offenders, and known test prerequisites.

The `tenex-edge-docs-queue` skill teaches repository discipline: GitHub Issues are the tactical queue, no new planning files, correct docs in place, classify generated wiki output, and retire stale planning material. <!-- [^019f1-9c16f] -->


The `tenex-edge` skill itself is resource-free: no `reference/` files or scripts are included, only the mental-model guidance and a small mechanics appendix. <!-- [^019f1-f17ce] -->
## Agent Safety

Some open issues are marked high-risk or agent-unsafe. Skills must teach agents when to proceed autonomously and when to stop for architecture or owner review. <!-- [^019f1-0029b] -->

## Skill File Discipline

Each skill keeps `SKILL.md` short and puts volatile details in `reference/` files. <!-- [^019f1-ad217] -->

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
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:019f12f9-8a0b-7012-ad2f-f4d0cb035d2b
---

# Tenex-Edge Skills

## Overview

This guide governs the family of `tenex-edge` agent skills written to `./skills/tenex-edge/` with symlinks from `~/.agents/skills/tenex-edge` and `~/.claude/skills/tenex-edge` pointing to the repo-local skill directory. The original skill taught agent operator commands (`who`, `chat`, `tail`, project groups, subgroup rooms, install, and keystore commands), but it has gone stale: self-identity now lives in `who` agent-context output, and targeting uses codename handles like `@brave-otter-417@laptop`. The current guide restructures that material into a set of focused skills, each kept short with volatile details in `reference/` files.

<!-- citations: [^019f1-7100b] [^019f1-106a9] -->
## Skill Set

The `tenex-edge-operator` skill teaches day-to-day fabric usage: `who`, `chat read/write`, `tail`, `agents`, `invite`, and `channels switch/create/list`. It also carries the rule to verify live help before trusting docs.

The `tenex-edge-host-integration` skill teaches hook wiring and debugging for Codex, Claude Code, and OpenCode: lifecycle mapping, trust gates, adapter logs, `TENEX_EDGE_BIN`, `TENEX_EDGE_HOME`, reinstall/codesign flow, and how to prove registration with `who`.

The `tenex-edge-verification` skill teaches local gates and test tiers: `just fmt-check`, `just loc-check`, `just lint`, `just test-unit`, when to use `cargo test`, when relay/croissant/nak are required, and how to run e2e safely. Because command and docs drift is already visible, this skill includes small scripts/resources such as a command that prints current help, LOC offenders, and known test prerequisites.

The `tenex-edge-docs-queue` skill teaches repository discipline: GitHub Issues are the tactical queue, no new planning files, correct docs in place, classify generated wiki output, and retire stale planning material. <!-- [^019f1-9c16f] -->


The `tenex-edge` skill itself is resource-free: no `reference/` files or scripts are included, only the mental-model guidance and a small mechanics appendix. <!-- [^019f1-f17ce] -->
## Agent Safety

Some open issues are marked high-risk or agent-unsafe. Skills must teach agents when to proceed autonomously and when to stop for architecture or owner review. <!-- [^019f1-0029b] -->

## Skill File Discipline

Each skill keeps `SKILL.md` short and puts volatile details in `reference/` files. <!-- [^019f1-ad217] -->

---
title: Mosaico CLI Commands
slug: mosaico-cli-commands
topic: mosaico
summary: Install the CLI with `just install`, then run `mosaico install --all`.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-13
verified: 2026-07-06
compiled-from: conversation
sources:
  - session:75f62bb9-f564-4633-8741-997dfea1d0e7
  - session:abce9e9f-8f3e-4561-9dd3-684afd59be80
  - session:019f5a74-0a91-7340-8299-8ac3dccfa36d
---

# Mosaico CLI Commands

## Installation

Install the CLI with `just install`, then run `mosaico install --all`. <!-- [^75f62-fdca2] -->

## Live View

The live view command is `mosaico who --live`. <!-- [^75f62-92209] -->

## Launch

The `mosaico launch` CLI command reattaches a live public session handle, resumes an exited handle when its harness supports it, or spawns a new agent harness (e.g. claude, codex) inside a detached pty session. <!-- [^abce9-05e58] -->

Agent launch configuration uses a `commands` array in `~/.mosaico/agents/<slug>.json`, with entries shaped as `{"name":"safe","argv":["claude"]}`.

`mosaico launch <agent> --command-name <name>` selects one configured command non-interactively. `-c/--command <command>` overrides the whole launch argv for that invocation. When multiple commands exist and no name is passed, launch opens an interactive picker. When no commands exist, interactive launch offers suggestions from other agents' `commands` entries, then built-in harness defaults, and saves the selected command back as `commands`.

## Command Tree

The public command tree follows a three-surface model: `mosaico who` for human read-only fabric overview, `mosaico my session` for agent full self/session briefing, and `mosaico sessions` for human interactive local session control. <!-- [^019f5-1fa80] -->

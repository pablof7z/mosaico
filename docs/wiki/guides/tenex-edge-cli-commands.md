---
title: Tenex-Edge CLI Commands
slug: tenex-edge-cli-commands
topic: tenex-edge
summary: Install the CLI with `just install`, then run `tenex-edge install --all`.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-03
verified: 2026-07-03
compiled-from: conversation
sources:
  - session:75f62bb9-f564-4633-8741-997dfea1d0e7
  - session:abce9e9f-8f3e-4561-9dd3-684afd59be80
---

# Tenex-Edge CLI Commands

## Installation

Install the CLI with `just install`, then run `tenex-edge install --all`. <!-- [^75f62-fdca2] -->

## Live View

The live view command is `who --live`. The old README's advertisement of a `tail` command is removed; `tail` is no longer part of the CLI, and a test asserts its absence. <!-- [^75f62-92209] -->

## Launch

The `tenex-edge launch` CLI command spawns an agent harness (e.g. claude, codex) inside a detached tmux session. <!-- [^abce9-05e58] -->

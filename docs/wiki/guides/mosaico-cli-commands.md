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

Each `~/.mosaico/agents/<slug>.json` selects one required `harness` bundle and an optional harness-specific `profile`. The bundle is defined in `~/.mosaico/harnesses.json` and owns the underlying harness, transport, and operational args. Profile application is code-owned per harness and transport: Claude PTY/headless uses `--agent <profile>`; Codex PTY/headless uses `--profile <profile>`; Codex app-server stages the selected `$CODEX_HOME/<profile>.config.toml` over the base config in an isolated `CODEX_HOME`, because app-server does not accept `--profile`. Codex custom-agent TOML is a separate concept and is not selected by this Codex config-profile mechanism. Unsupported harness/transport/profile combinations fail loudly; an absent profile uses the harness-native default.

`mosaico launch <agent>` uses that configuration directly. Launch-time command, bundle, and named-command overrides do not exist, and missing bundles fail instead of selecting a built-in fallback.

## Command Tree

The public command tree follows a three-surface model: `mosaico who` for human read-only fabric overview, `mosaico my session` for agent full self/session briefing, and `mosaico sessions` for human interactive local session control. <!-- [^019f5-1fa80] -->

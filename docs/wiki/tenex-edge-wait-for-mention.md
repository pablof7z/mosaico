---
title: Tenex-Edge Wait-for-Mention
slug: tenex-edge-wait-for-mention
topic: tenex-edge
summary: The `wait-for-mention` command polls the SQLite inbox every ~500ms until a mention arrives
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:3da7f7d8-c5a3-4065-be64-3a3a73dbb1d6
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
---

# Tenex-Edge Wait-for-Mention

## Polling and Inbox Behavior

The `wait-for-mention` command polls the SQLite inbox every ~500ms until a mention arrives. On startup, it performs a self-fetch from the relay (same as `inbox` does) to handle the engine warmup race. When a mention arrives, the command drains the inbox, prints the mention, prints a re-run reminder, and exits with code 0. It reuses the existing `fetch_mentions_into_inbox` and `drain_inbox` logic. <!-- [^3da7f-6] -->

The command supports an optional `--timeout` parameter (default 5 minutes) that causes a clean exit on timeout so no forgotten background processes linger. <!-- [^3da7f-7] -->

The re-run reminder instructs the agent to run the command with `run_in_background=true`, matching the mechanism used to launch it. <!-- [^3da7f-8] -->


A streaming mention subscription (long-lived NDJSON command) is provided as a generic seam, with wait-for-mention remaining as the portable floor for all harnesses. <!-- [^162f9-21] -->
## Background Execution and Agent Wake-Up

The `wait-for-mention` command is run by the agent using `run_in_background=true` (not shell `&`), so the harness tracks the process and wakes the idle agent on completion. An idle agent wakes up when a background command it launched exits, allowing `wait-for-mention` to re-activate an idle agent upon receiving a mention. <!-- [^3da7f-9] -->

## Hook Injection

The instruction to run `wait-for-mention` is injected via the `UserPromptSubmit` hook exactly once per session (tracked using a temp flag file keyed on `sid`), so the agent is guaranteed to be in an active turn and can immediately execute the command. The instruction is NOT injected at session start (via `SessionStart` hook), because the agent is idle on the welcome screen with no LLM call to act on the instruction until a user prompt triggers a turn. Hook scripts for all three harnesses (Claude Code, Codex, OpenCode) inject the instruction. <!-- [^3da7f-10] -->

## Harness Configuration

All harness configurations point directly to the hook scripts in the source tree, avoiding divergence from a separate deployed copy. <!-- [^3da7f-11] -->

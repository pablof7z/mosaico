---
title: Mosaico Launch
slug: mosaico-launch
topic: mosaico
summary: When `mosaico launch` spawns an agent harness in pty, `default-terminal` and `terminal-overrides` are set globally (`-g`) before `new-session` forks the chi
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-07
verified: 2026-07-07
compiled-from: conversation
sources:
  - session:abce9e9f-8f3e-4561-9dd3-684afd59be80
  - session:fea5307b-d9a0-46fe-977c-408e5e0e0ff4
---

# Mosaico Launch

## Named Commands

Agent launch commands are stored as named entries in `~/.mosaico/agents/<slug>.json` under `commands`, for example `{"commands":[{"name":"full","argv":["claude","--dangerously-skip-permissions"]}]}`.

A bare `mosaico launch <agent>` first checks whether `<agent>` is an existing public session handle. A live PTY is reattached; an exited session with a native resume token is resumed. Otherwise it chooses the command before calling the daemon. A single configured command launches directly. Multiple configured commands open a TTY picker unless `--command-name <name>` selects one explicitly. `-c/--command <command>` remains a one-shot full argv override. If no commands exist, interactive launch suggests commands from other agents' `commands` entries with conservative slug/path adaptation; if no local suggestions exist, it suggests built-in harness commands.

## Initial Prompt

`mosaico launch <agent> "prompt"` spawns the PTY session, waits for the harness input to become interactive, then injects and submits the prompt as the opening user turn before attaching the terminal. Omit the prompt for the normal clean interactive launch. Use `-` as the prompt value to read the opening prompt from stdin.

## pty Environment Inheritance

When `mosaico launch` spawns an agent harness in pty, `default-terminal` and `terminal-overrides` are set globally (`-g`) before `new-session` forks the child process so the child inherits `pty-256color` and color support from the first frame. The `terminal-overrides` value is `*:Tc:RGB:extkeys`.

With these settings in place, the fixed `mosaico launch` window renders agent harness colors identically to a direct launch — including the colored robot logo, orange banner, and yellow warning.

The daemon allocates the session key before spawn and injects
`MOSAICO_PUBKEY` from process birth. PTY endpoint IDs, harness-native resume
tokens, sockets, and PIDs remain typed runtime locators owned by that pubkey;
none of them is a second session identity.

The `mosaico launch` status line displays the published agent name, work-root
channel, current channel, optional distilled title, and live state. For example:
`amber-claude mosaico support [Refactoring the inbox] [writing tests]`.

<!-- citations: [^abce9-cad96] [^abce9-0fd96] [^abce9-2b259] [^abce9-b1683] -->

## Mention Injection

Mention injection into a `mosaico launch` agent must occur in the `userPrompt` hook. <!-- [^fea53-66223] -->

## Agent Roles

Agent1 is a raw `claude` session (direct mode, host-pty-observed but not daemon-anchored) that the user types the initial instruction into. Agent2 is a `claude` session launched via `mosaico launch` that is expected to receive live daemon-pushed, attributed mentions. <!-- [^fea53-1e000] -->

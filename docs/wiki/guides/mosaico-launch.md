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

## Configured Harness

An agent file selects a required harness bundle and an optional harness-specific profile, for example `{"slug":"reviewer","harness":"yolo-claude","profile":"reviewer"}`. The corresponding `harnesses.json` bundle owns the underlying harness, transport, and operational args: `{"yolo-claude":{"harness":"claude","transport":"pty","args":["--dangerously-skip-permissions"]}}`. Claude PTY/headless applies the profile with `--agent`; Codex PTY/headless applies it with `--profile`; Codex app-server stages `$CODEX_HOME/<profile>.config.toml` into an isolated home. Codex custom-agent TOML is a separate concept and is not selected by the Codex config-profile mechanism. Unsupported combinations fail loudly; an absent profile uses the harness-native default.

A bare `mosaico launch <agent>` first checks whether `<agent>` is an existing public session handle. A live PTY is reattached; an exited session with a native resume token is resumed. Otherwise the daemon resolves the agent-selected bundle exactly. The executable and profile mechanism are code-owned by the `(harness, transport)` driver; the bundle contributes operational args. There are no command pickers, launch-time command or bundle overrides, or built-in bundle fallbacks.

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

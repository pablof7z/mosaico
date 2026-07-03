---
title: Tenex-Edge Launch
slug: tenex-edge-launch
topic: tenex-edge
summary: When `tenex-edge launch` spawns an agent harness in tmux, `default-terminal` and `terminal-overrides` are set globally (`-g`) before `new-session` forks the chi
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-03
verified: 2026-07-03
compiled-from: conversation
sources:
  - session:abce9e9f-8f3e-4561-9dd3-684afd59be80
---

# Tenex-Edge Launch

## tmux Environment Inheritance

When `tenex-edge launch` spawns an agent harness in tmux, `default-terminal` and `terminal-overrides` are set globally (`-g`) before `new-session` forks the child process so the child inherits `tmux-256color` and color support. The `terminal-overrides` value uses the format `*:Tc:RGB:extkeys` (term-pattern-prefixed capability tokens), not the previous orphaned format `,*:Tc,RGB,extkeys`. <!-- [^abce9-cad96] -->

`@te_session` is a tmux session option stamped by the daemon that the agent harness expects to be set; when unset, the tmux status line shows a red `@te_session not set` error. <!-- [^abce9-0fd96] -->

The `tenex-edge launch` status line displays the agent identifier and session state in the format `<agent>@<host> <project> <project> [idle]` (e.g. `claude@isolated-test-mac project project [idle]`). <!-- [^abce9-2b259] -->

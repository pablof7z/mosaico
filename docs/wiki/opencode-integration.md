---
title: OpenCode Integration
slug: opencode-integration
topic: opencode-integration
summary: The opencode integration injects the stdout from the `user-prompt-submit` hook verbatim at turn start, rather than rebuilding context blocks in TypeScript
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-16
updated: 2026-06-16
verified: 2026-06-16
compiled-from: conversation
sources:
  - session:9337d29e-ac62-417c-8e99-0cc22cbbfad3
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
---

# OpenCode Integration

## Context Injection

The opencode integration injects the stdout from the `user-prompt-submit` hook verbatim at turn start, rather than rebuilding context blocks in TypeScript. It maps its mid-turn `transform` calls to the `post-tool-use` hook (the non-destructive peek path) and injects that stdout, instead of making bespoke `tenex-edge inbox` and `tenex-edge who` shell-outs. <!-- [^9337d-2] -->


OpenCode uses a TypeScript plugin for integration (experimental.chat.messages.transform for injection, tool.execute.after for observation, event session.created/idle for lifecycle). <!-- [^f3a73-1] -->
## Removed Code

The hand-built `selfLine` and `hinted` one-shot flag, the `tenex-edge inbox` shell-out, the `tenex-edge who` shell-out, the `run()` and `stripAnsi()` helpers, and the `SHORT_CODE` capture from session-start are removed from the opencode integration. <!-- [^9337d-3] -->

## Hook Split

The opencode integration uses the drain-once (`user-prompt-submit`) / peek-mid-turn (`post-tool-use`) split for context awareness, matching the behavior of Claude Code and Codex. <!-- [^9337d-4] -->

## Codex Integration

Codex hooks exist (same event names as Claude Code: SessionStart, UserPromptSubmit, PostToolUse, Stop) in ~/.codex/config.toml, but they only fire in the interactive TUI, not in codex exec; the reliable Codex integration is the tenex-codex launcher wrapper. <!-- [^f3a73-2] -->

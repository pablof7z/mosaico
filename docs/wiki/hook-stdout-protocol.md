---
title: Hook Stdout Protocol
slug: hook-stdout-protocol
topic: hook-protocol
summary: The canonical source of truth for the self-identity line, inbox, project chat, and peer roster is the Rust `turn.rs` hook, which all integrations (Claude Code,
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
---

# Hook Stdout Protocol

## Hook Stdout Protocol

The canonical source of truth for the self-identity line, inbox, project chat, and peer roster is the Rust `turn.rs` hook, which all integrations (Claude Code, Codex, opencode) consume via the hook's stdout. <!-- [^9337d-1] -->

---
title: Tenex-Edge Thread Storage
slug: tenex-edge-thread-storage
topic: tenex-edge
summary: The thread dual-write infrastructure (local SQLite read-model of relay conversations via `projects`, `threads`, `messages`, `message_recipients` tables) was rem
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-10
updated: 2026-06-10
verified: 2026-06-10
compiled-from: conversation
sources:
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
---

# Tenex-Edge Thread Storage

## Thread Storage History

The thread dual-write infrastructure (local SQLite read-model of relay conversations via `projects`, `threads`, `messages`, `message_recipients` tables) was removed from the codebase prior to the `98582fa` refactor. <!-- [^56f9f-9] -->

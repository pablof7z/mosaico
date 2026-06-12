---
title: OpenCode Configuration
slug: opencode-configuration
topic: tenex-edge
summary: The @opencode-ai/plugin dependency version must match the opencode binary version (1.16.2) in both ~/.config/opencode/package.json and ~/.opencode/package.json.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-08
updated: 2026-06-09
verified: 2026-06-08
compiled-from: conversation
sources:
  - session:96aedf14-df2c-425b-b548-0fa7d1c1ba63
  - session:956595fb-fa6a-45f8-869c-b53cae16124f
  - session:ab9998c4-6e65-410e-b298-122a2072171c
---

# OpenCode Configuration

## Dependency Version

The @opencode-ai/plugin dependency version must match the opencode binary version (1.16.2) in both ~/.config/opencode/package.json and ~/.opencode/package.json. Testing must also include the opencode agent adapter (TS plugin-based integration) alongside claude-code and codex.

<!-- citations: [^96aed-6] [^96aed-7] [^95659-3] [^ab999-25] -->

## Session Hooks

OpenCode's `session.idle` hook must be verified to fire per-turn rather than mid-loop to prevent premature idle states during long turns. In headless `opencode run` mode, the plugin's fire-and-forget session-start races the single turn, so the session must be pre-registered deterministically via the hook. <!-- [^ab999-26] -->

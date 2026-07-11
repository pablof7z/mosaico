---
title: Tenex-Edge Who Rendering
slug: tenex-edge-who-rendering
topic: tenex-edge
summary: `tenex-edge who` renders terminal text for operators and structured workspace awareness XML for exact agent sessions.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-11
verified: 2026-07-11
compiled-from: conversation
sources:
  - session:7d6bf2fe-8dc9-4bd0-aeeb-de1827bf68d1
---

# Tenex-Edge Who Rendering

## Audience-Aware Rendering

An exact live agent session receives XML with `<self>`, a global `<agents>` capability inventory, and `<workspaces>`. A bare operator receives terminal-oriented text. Both are read projections over the daemon-owned store; neither renderer queries the relay directly.

Every known workspace appears in agent XML. The workspace is its root channel,
so `<workspace channel="workspace" ... members="N">` carries root membership
directly and only real descendants render as `<channel>` rows. By default only
the caller's workspace is expanded; `--all-workspaces` expands every workspace
the caller has joined. Channel contents recurse only while the caller is a
member of each parent channel. Member counts exclude backend keys.

<!-- citations: [^7d6bf-cad7a] [^7d6bf-5aab0] -->
## Capability Availability

The global `<agents>` inventory groups kind:30555 advertisements by backend and role. Remote roles render as `slug@backend`, retain their `about` criteria, and list every advertised workspace in `workspace-availability`. These are spawnable capabilities, not channel members.

<!-- citations: [^7d6bf-bafea] [^7d6bf-db5d3] -->

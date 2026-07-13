---
title: Tenex-Edge My Session Command
slug: tenex-edge-my-session
topic: tenex-edge
summary: "The `my session` command gives an agent a full self/session briefing across identity, joined channels, workspaces, members, and capabilities."
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-13
updated: 2026-07-13
verified: 2026-07-13
compiled-from: conversation
sources:
  - session:019f5a74-0a91-7340-8299-8ac3dccfa36d
---

# Tenex-Edge My Session Command

## `my session` Overview

The `my session` command gives an agent a full self/session briefing: who the
agent is, the workspace it runs from, the channels it has joined, all
workspaces it knows, and its agent capabilities. Sessions appear in this same
self/channel briefing because they are channel members in tenex-edge; there is
no parallel session inventory. `my session` is a pure read that does not
advance the hook cursor. <!-- [^019f5-a7eb2] -->

The briefing expands every workspace and channel the session has joined, while keeping merely-known workspaces compact. The existing agent renderer is reused intact for `my session`, projecting `<self>`, global agent capabilities, every workspace, and channel-member sessions with state, status, and seen information. <!-- [^019f5-1cc5f] -->

## Self-Management Grammar

The self-management grammar is consolidated under `tenex-edge my session`:

- `tenex-edge my session` — full self/session briefing.
- `tenex-edge my session status <title>` — set the status of the agent.
- `tenex-edge my session end --self` — end the session.
- `tenex-edge my session kill --self` — kill the session.
- `tenex-edge my session pty-wrap-me --self` — wrap a PTY.

<!-- citations: [^019f5-ec07f] [^019f5-9e8cc] -->

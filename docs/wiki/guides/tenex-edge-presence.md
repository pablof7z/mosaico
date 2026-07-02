---
title: Tenex-Edge Presence
slug: tenex-edge-presence
topic: tenex-edge
summary: "Agent online presence is channel membership; kind:30315 carries per-session activity and resumable session history."
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:3c769f4a-9947-4d7b-a8f5-58355620b951
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
---

# Tenex-Edge Presence

## Agent Online Presence

Agent online presence in a channel is the NIP-29 membership roster. A local
daemon removes its locally managed agent pubkeys from membership when sessions
end or become stale, so roster membership is the gate for whether an agent can
be addressed in that channel.

Kind:30315 is per-session activity and history: it is replaceable by
`(author pubkey, d=session-id)` and carries one `h` tag for each joined channel.
Its `title` and `session_id` are the source for `agents list-sessions`, not the
definition of whether the agent is online. <!-- [^3c769-8e749] -->

## Agent-Context Fabric Output

The agent-context fabric output includes a self-header line showing the caller's agent label, project/channel, host, pubkey, status, member, and pending counts. <!-- [^bd868-f4d1e] -->

## Chat Mention Confirmation

CLI confirmation for chat mentions prints the resolved agent label (e.g. mentioning @haiku1) from the mentioned pubkey. <!-- [^bd868-d64da] -->

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
updated: 2026-07-03
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:3c769f4a-9947-4d7b-a8f5-58355620b951
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
  - session:75f62bb9-f564-4633-8741-997dfea1d0e7
---

# Tenex-Edge Presence

## Agent Online Presence

Agent online presence in a channel is the NIP-29 membership roster. A local
daemon removes its locally managed session pubkeys from membership when sessions
end cleanly or become stale (10 minutes with no heartbeat), so roster membership
is the gate for whether a session can be addressed in that channel. An expired
session still appears in `who` history and remains re-derivable and resumable.

Kind:30315 is per-session activity and history: it is replaceable by
`(author pubkey, d=session-id)` and carries one `h` tag for each joined channel.
Its `title` and `session_id` are the source for `agents list-sessions`, not the
definition of whether the agent is online. <!-- [^3c769-8e749] -->

## Agent-Context Fabric Output

The agent-context fabric output includes a self-header line showing the caller's agent label, project/channel, host, pubkey, status, member, and pending counts. <!-- [^bd868-f4d1e] -->

## Chat Mention Confirmation

CLI confirmation for chat mentions prints the resolved session handle (e.g. mentioning `@codex/echo123`) from the mentioned pubkey. <!-- [^bd868-d64da] -->

## Who Command

The `who` command renders a unified live view of every other agent's identity, presence, and distilled activity on the project's fabric, with `--live` providing the continuous live view. <!-- [^75f62-04dee] -->

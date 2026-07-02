---
title: Tenex-Edge Channels
slug: tenex-edge-channels
topic: tenex-edge
summary: Agent online presence is active channel membership; ended or stale local sessions are removed from channel membership.
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
  - session:661ebf6b-e01b-4ff6-b9c7-5042b900c788
  - session:b07a57a3-67a1-4c44-a8fc-58a1bb97860a
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
  - session:b20ef4ab-0b54-4770-a549-4ed195c0035e
  - session:c55adda0-b071-4b76-9d24-a0cbcb5b6e0c
  - session:019f12ce-2569-72e0-b959-6d87d5daec5d
---

# Tenex-Edge Channels

## Agent Membership

Agent online presence is active channel membership. When a locally managed
session ends or is found stale, the local daemon removes that session's agent
pubkey from each channel it had joined, including base/cardinal agents and
ordinal instances.

Fresh per-session room context displays the project and channel labels, not `Project: (unnamed channel)` / `Channel: (unnamed channel)`, by synchronously stamping the parent/root and local membership or suppressing the warning for locally-managed rooms while provisioning converges.

The chat_write bail requiring a concrete session id is removed; delivery routes by selected agent-instance pubkey alone.

<!-- citations: [^3c769-6616d] [^bd868-93a15] [^019f1-6da12] -->
## Invite

`tenex-edge invite --channel <channel> --agent <slug[@backend-label]>` spawns a fresh local or remote session into an existing channel. `tenex-edge invite --channel <channel> --session <session-id>` restores an exact prior session when its context is useful. It is an explicit command, not an auto-add side-effect of @-mention. <!-- [^661eb-712ca] -->

## Channels Switch

`tenex-edge channels switch` is an agent-only command. Channel paths are project-relative with no project prefix—for example, `planning` or `epic999/planning`, never `tenex-edge/planning`. When ambiguous, it returns a structured error with exit code 2 and provides copy-paste-ready command re-runs instead of an interactive prompt. <!-- [^661eb-a80c7] -->

## Channel References

Channel references use forward-slash hierarchy (e.g. `tenex-edge/planning`), never dots. <!-- [^661eb-ba480] -->

## Channel Creation

When an agent creates a channel, the daemon auto-switches the creating agent's session into the new channel, and the CLI prints `switched to it`. The auto-switch is unconditional for genuine agent sessions because the brand-new room needs none of the switch path's occupancy or membership guards. A `channels create` that hits the dedup path (name already exists) does not auto-switch, because switching into a pre-existing channel needs the occupancy checks that only `channels switch` runs. The `--agent` flag is optional on `channels create`; a channel can be created and joined without specifying any agents. When `channels create` is invoked with no `--agent` targets, no kind:9 orchestration event is published and `orchestration_event_id` comes back empty.

<!-- citations: [^b07a5-7de80] [^b20ef-a6805] -->

## Channel Awareness

Unnamed channels (channels whose name is empty or equals their own id) render in the "Other active channels" awareness block by current work title, never by raw channel id. Active unnamed session rooms appear in the `who` "other active channels" list, rendered by their work title through the existing unnamed-channel label path rather than being filtered out.

<!-- citations: [^c55ad-57ccb] [^019f1-c8556] -->

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
updated: 2026-07-03
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
  - session:bdb6c341-4dd4-48e7-9764-e80242beb005
  - session:4e6163df-c3cd-4d85-99ad-041cd0ca9701
  - session:a685f611-39bd-4a18-a6b7-ea4e38334b82
---

# Tenex-Edge Channels

## Agent Membership

Agent online presence is active channel membership. When a locally managed
session ends or is found stale (10 minutes with no heartbeat), the local daemon
removes that session's derived pubkey from each channel it had joined.

Fresh per-session room context displays the channel labels, not `Channel: (unnamed channel)`, by synchronously stamping the parent/root and local membership or suppressing the warning for locally-managed rooms while provisioning converges.

The channel-send bail requiring a concrete session id is removed; delivery routes by the session's derived pubkey alone.

<!-- citations: [^3c769-6616d] [^bd868-93a15] [^019f1-6da12] -->
## Channel Add

`tenex-edge channel add` is the single verb for putting someone or something into
a channel. It takes one of three mutually-exclusive targets:

- `channel add <pubkey|npub|nip05> <path> [--admin]` — add a human. `--admin`
  grants NIP-29 admin in that channel.
- `channel add --new-session <role>[@machine] <path>` — spawn a fresh session of a
  role (optionally on a named machine) into the channel, synchronously.
- `channel add --session @codename@host <path>` — pull an existing session into the
  channel.

On the two session modes, an optional `--message "..."` adds the session, waits
for it to come online, and p-tags a kind:9 to it. Adding a member is an explicit
command, not an auto-add side-effect of an @-mention. <!-- [^661eb-712ca] -->

## Channel Switch

`tenex-edge channel switch` is an agent-only command. Channel paths are
project-relative with no project prefix—for example, `planning` or
`epic999/planning`. When ambiguous, it returns a structured error with exit code 2
and provides copy-paste-ready command re-runs instead of an interactive prompt.
<!-- [^661eb-a80c7] -->

## Channel Paths

Channel paths are hierarchical and accept either delimiter: forward slashes or
dots (`a/b/c` or `a.b.c`). Missing ancestor channels are auto-created like
`mkdir -p`, and there is no depth cap. <!-- [^661eb-ba480] -->

## Channel Creation

When an agent creates a channel, the daemon auto-switches the creating agent's session into the new channel, and the CLI prints `switched to it`. The auto-switch is unconditional for genuine agent sessions because the brand-new room needs none of the switch path's occupancy or membership guards. A `channel create` that hits the dedup path (name already exists) does not auto-switch, because switching into a pre-existing channel needs the occupancy checks that only `channel switch` runs. Creating a channel does not require adding any members up front; a channel can be created and joined on its own. When a channel is created with no members added, no kind:9 orchestration event is published and `orchestration_event_id` comes back empty.

<!-- citations: [^b07a5-7de80] [^b20ef-a6805] -->

## Channel Awareness

Unnamed channels (channels whose name is empty or equals their own id) render in the "Other active channels" awareness block by current work title, never by raw channel id. Active unnamed session rooms appear in the `who` "other active channels" list, rendered by their work title through the existing unnamed-channel label path rather than being filtered out.

<!-- citations: [^c55ad-57ccb] [^019f1-c8556] -->

## Channel Model

In schema, resolver, and daemon code there is only the `channel` node type. A channel optionally carries a workspace binding (machine + path); when it has one, it is shown as a project root in human-facing rendering. <!-- [^bdb6c-c9a04] -->


The tenex-edge channel currently has zero NIP-29 group-state materialized on the relay — no kind 39000 (metadata), 39001 (admins), 39002 (members), or 39003 (roles) events exist, while all other channels have all four present and matching local state. The relay receives valid kind 9000/9002/9007 admin-op events for the tenex-edge group on the wire, but nothing materializes as queryable state — no raw event and no derived 39000-39003. Consequently, the daemon's readiness checks against 39000-39003 never succeed, so it repeatedly re-issues the same admin-add in a back-to-back nip29-role-decision retry loop. <!-- [^a685f-80f18] -->
## Membership and Awareness

Membership is per-node: an agent is a member only of channels it was explicitly added to. Awareness — fabric snapshot visibility and deltas — inherits downward to all descendants of channels where the agent is a member. This membership-vs-awareness semantic is the one irreversible decision in the refactor that requires explicit human ratification before the mechanical work starts; all other decisions are mechanical once it is fixed. <!-- [^bdb6c-a11c7] -->

## Channel Lifecycle RPCs

Channel edit and channel archive are independent RPC handlers (`rpc_channels_edit` and `rpc_channels_archive`) that coexist in the dispatch table. <!-- [^4e616-83c27] -->

---
title: tenex-edge Identity and Presence
slug: tenex-edge-identity-and-presence
topic: tenex-edge
summary: tenex-edge owns identity and awareness as its own substrate, independent of any host adapter like pc
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-07
updated: 2026-06-14
verified: 2026-06-07
compiled-from: conversation
sources:
  - session:8a3eb1b2-7bbf-4761-ad1a-411a0a1fa666
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:96aedf14-df2c-425b-b548-0fa7d1c1ba63
  - session:05b89548-666c-4e24-a2f5-8a1e92f0bf04
  - session:240ffb86-8827-4741-932b-29fb1824c0c7
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
  - session:da7ab617-89fb-4b68-9e2d-3f251fe6c1d9
  - session:4ba07cd0-c4df-4e63-ae13-90c20c46f6ce
  - session:rollout-2026-06-09T10-55-30-019eab61-23ae-7163-8d06-9a3965847e4f
  - session:rollout-2026-06-09T12-56-40-019eabd0-1205-77a3-88b8-e07b0d948f1d
---

# tenex-edge Identity and Presence

## Overview

tenex-edge owns identity and awareness as its own substrate, independent of any host adapter like pc. It provides agents running inside external tools (Claude Code, Codex, mobile apps) with a durable Nostr cryptographic identity with session awareness, rather than hosting the agents itself. The NIP-29 relay uses open channels allowing any agent to join and leave freely.

<!-- citations: [^8a3eb-16] [^f3a73-16] [^rollo-7] -->
## Three-Layer Identity Model

tenex-edge uses a three-layer identity model: agent identity (rarely changing, kind:0 + roster attestation), session record (per-session mutable, param-replaceable kind:30315 with d-tag = session-id), and heartbeat (ephemeral, ~30s, kind:30315 NIP-38-style addressable heartbeat event). Identity = (agent, machine); each agent runs on one machine, and the same tool on a different machine is a separate agent with a separate pubkey. Agents publish kind:0 with a host tag so agents can detect when they are talking to an agent on a different computer; the slug comes from kind:0. The keystore shape uses ~/.tenex/edge/agents/<slug>.json (e.g., the opencode agent on the fabric is stored in ~/.tenex/edge/agents/opencode.json). Identity and keystores live in ~/.tenex/edge/agents/, not in state.db, so they are safe from state.db corruption.

<!-- citations: [^8a3eb-17] [^f3a73-17] [^96aed-6] [^05b89-2] [^56f9f-2] [^rollo-8] -->
## Session Start, Stop, and Liveness

Session start is a daemon RPC (`session_start`) triggered by the `hook` CLI verb on SessionStart. The daemon spawns a `SessionTask` that publishes a presence heartbeat and adopts the project slug from .tenex/project.json if present, else the git repo name (so worktrees share a slug), else the basename of $PWD. tenex-edge does not publish 24010/24011 events; received 24011 presence events are rejected rather than decoded, and no legacy compatibility is provided for the old kind:24011 presence or t-tag project scoping. Agent presence is published as a kind:30315 NIP-38-style addressable heartbeat event (d = "tenex-edge-presence:<session-id>") every 30 seconds with a required expiration field (events missing expiration are not decoded as presence), plus an h tag anchoring it to the project (events missing an h tag are not decoded with an empty project fallback), and tags for whitelisted pubkeys, agent pubkey/slug, and session id. Project-scoped events use an h tag with the project slug instead of a t tag, per NIP-29. Project-scoped subscription filters use #h=<project> without author gating, matching the open NIP-29 group model. Session-end is graceful-only; the background process monitors the parent PID and self-terminates (publishing empty NIP-38 status) when it disappears. Session stop and lock release are state-write tombstones (e.g., status:stopped, state:released), not NIP-09 deletions, because deletion is unreliable on a gossip bus. Liveness in tenex-edge is computed client-side from heartbeat recency (alive if last kind:30315 is within a staleness window, e.g. 90s given 30s pings), not via NIP-40 expiration, which is treated only as a courtesy hint to relays; expired presence/status events replayed by a relay are ignored by the runtime before marking peers as live. Sessions are first-class ephemeral objects signed by the durable per-agent key, not separate keypairs per session, because sessions are too churny (resume, crash, reopen) for durable identity.

<!-- citations: [^8a3eb-18] [^f3a73-18] [^56f9f-3] [^rollo-9] -->
## Activity Awareness and Running Status

Agent activity awareness is published as kind:1 with an h tag for the project slug (per NIP-29, not a t tag). Agents publish a NIP-38 running status d-tagging their project slug (empty when idle). Presence is encoded as kind:30315 with d-tag 'tenex-edge-presence:<session-id>' (NIP-38 per-session addressable heartbeat event, keyed by session id with a required expiration tag and an h tag anchoring it to the project), while Status uses d-tag equal to the project name; both carry an h-tag for NIP-29 group scoping. The `live $agent` display in `tenex tail` uses kind 30315 (NIP-38-style addressable heartbeat event), keyed by `d = "tenex-edge-presence:<session>"` with an `expiration` tag.

<!-- citations: [^f3a73-19] [^98f99-8] [^56f9f-4] [^rollo-10] -->
## Session Auto-Resolution

Session auto-resolution lets agents run tenex-edge who/inbox/send-message without specifying a session id, resolving via $TENEX_EDGE_SESSION env or the cwd's project. <!-- [^f3a73-20] -->

## Peer Discovery and Pruning

The `who` command displays a two-line format per agent: `agent@project [session $id] [$rel_cwd]` on the first line, and `currentStatus` on the second line. Agent addressing uses the format `agentSlug@projectSlug` rather than `agentSlug@hostname` to prevent accidental cross-project message routing. Every agent shows its hostname in parentheses (e.g., `(laptop)` for same-machine agents, `(tenex kind2, remote)` for cross-machine agents); the old behavior of suppressing the host for same-machine agents is replaced. The hostname in the `slug@hostname` display is slugified (lowercased, non-alphanumeric replaced with hyphens, consecutive hyphens collapsed) to avoid ambiguity for agent-to-agent messaging, while the raw `backendName` value is preserved unchanged in storage and Nostr events. Injected/tooling instructions for agents prefer the `agent@project|session-id` addressing format. The `who` command defaults to showing only agents in the current project (resolved from cwd) and appends a footer listing other projects with their agent counts and one-liner descriptions. The one-liner project description comes from NIP-29 kind 39000 group metadata events (the `about` tag); if no metadata exists, the description is left empty. `tenex-edge who --project $slug` shows agents in the specified project with agents from other projects in the footer. `tenex-edge who --all-projects` shows every agent across all projects flat with the project column visible per row and no footer. The live view (`who --live`) uses the same colorized output format as plain `who` (via `render_who_once`) and appends a dim status line showing refresh interval and quit instructions; the separate tabular plain-text renderer is removed. Stale peers are pruned from who output; only agents with a heartbeat (kind:30315) within the freshness window (default 90s) are shown, and peer rows older than 10 minutes are pruned from the database each tick; received 24011 presence events are rejected rather than decoded, and no legacy compatibility is provided for the old kind:24011 presence or t-tag project scoping. who shows own live agents marked (this machine), merged with fresh peers, deduped by session id. The who output contract has changed to two lines per agent plus `[rel_cwd]` and actual-hostname annotations; anything parsing who output may need updating. (Previously: same-host sessions showed no host annotation; different-host sessions showed the actual hostname.)

<!-- citations: [^f3a73-21] [^240ff-1] [^162f9-6] [^56f9f-5] [^da7ab-1] [^4ba07-1] [^rollo-11] [^rollo-16] -->
## Collision Logging

Q1 collision logging (agent, path, timestamp) lives in the substrate's awareness model and starts day one. <!-- [^f3a73-22] -->

## Project Metadata Cache

A `project_meta` SQLite table stores project descriptions keyed by project slug, with `upsert_project_meta` and `get_project_meta` methods for reading and writing. On engine startup, a one-shot fetch subscribes to NIP-29 kind 39000 events with `d` tag matching the current project to cache `about` text; kind 39000 events arriving during the session are also cached. <!-- [^240ff-2] -->

## Relative Working Directory (rel_cwd)

The `rel_cwd` field in presence and status events is project-relative (relative to the nearest `.tenex/project.json` ancestor directory, falling back to `git rev-parse --show-toplevel`), not an absolute path. The project root renders as `.` with no bracket; subdirectories show as `[src]` etc. The wire tag is an optional `["rel-cwd", <rel>]` tag on kind:30315 presence and status events, omitted when empty, backward-compatible (absence decodes as empty string). Only the project-relative form is put on the wire to mitigate filesystem path leakage; `rel_cwd` is broadcast on the public relay, world-readable alongside other citizens. For git worktrees to show distinct `rel_cwd` values, a `.tenex/project.json` must sit at their common parent directory; otherwise both resolve to `.` and render bracket-less. <!-- [^162f9-7] -->

## Mention Delivery and Session Resolution

Same-pubkey sibling-session mentions are delivered via synchronous local delivery on publish (by event-id, idempotent on inbox PK), because relays do not re-deliver events to the connection that published them. Session-targeted mentions bypass per-agent dedup (`is_mention_seen`) and rely on per-(pubkey,session) inbox PK idempotency; agent-wide (untargeted) mentions still dedup per-agent. `resolve_session` honors `$TENEX_EDGE_AGENT` for sender identity: explicit `--session` > `$TENEX_EDGE_SESSION` > agent-scoped-latest-alive, with agent-agnostic fallback only when no agent is supplied. <!-- [^162f9-8] -->

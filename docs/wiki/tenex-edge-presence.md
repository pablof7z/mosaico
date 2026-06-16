---
title: Tenex-Edge Presence
slug: tenex-edge-presence
topic: tenex-edge
summary: tenex-edge does not publish 24010/24011 events; received 24011 presence events are ignored, not emitted
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-10
updated: 2026-06-16
verified: 2026-06-10
compiled-from: conversation
sources:
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
  - session:da7ab617-89fb-4b68-9e2d-3f251fe6c1d9
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:240ffb86-8827-4741-932b-29fb1824c0c7
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:rollout-2026-06-09T10-55-30-019eab61-23ae-7163-8d06-9a3965847e4f
  - session:rollout-2026-06-09T15-01-20-019eac42-32f0-7ff0-bda2-da2de3b78ed7
  - session:1b868736-ed6b-4f88-84d9-26bb320accfd
---

# Tenex-Edge Presence

## Tenex Edge Presence

tenex-edge does not publish 24010/24011 events; received 24011 presence events are ignored, not emitted. Legacy kind 24011 presence events and t-only project events are not decoded and are ignored (no legacy compatibility). Presence is published as a recurring heartbeat with kind 30315 and is anchored to the project via the h tag. A d tag of "tenex-edge-presence:<session-id>" prevents sessions of the same agent from clobbering each other (NIP-38 replaceable per-session heartbeat), an h tag for the project (NIP-29 group scoping), p tags for each whitelisted human, plus agent, session-id, host, optional rel-cwd, and expiration tags. One deterministic derive_status(state, now) -> DerivedStatus projection is shared by the publisher, who (both local and peer branches), and the turn-context delta, eliminating the local-vs-peer fork. The kind:30315 event must carry a NIP-40 ["expiration", <ts>] tag set to now + status_ttl (90s) on each heartbeat; when the session stops heartbeating, the event expires off the relay, so absence = dead (Previously: events had a never-expire policy). No tombstone or terminal lifecycle event is published on session end; the NIP-40 expiration tag on the heartbeat is the sole liveness mechanism. The runtime publishes expiring presence heartbeats on start, every heartbeat interval, and clean exit. The runtime ignores expired presence and status relay replays before marking peers as live. Already-running __run-session processes must be restarted to publish the new h/kind:30315 wire shape. The `live $agent` display in `tenex tail` shows a `DomainEvent::Presence` derived from kind 30315 (NIP-38-style addressable heartbeat event), keyed by `d = "tenex-edge-presence:<session>"` with an `expiration` tag. Stale peer sessions are pruned: who only shows peers whose heartbeat is still fresh (within 90s); the engine prunes peer rows older than 10 minutes each tick; --all shows stale ones. Remote agents in the who command display their actual hostname (e.g., (tower)) instead of the generic (remote) string. The who command displays agents using the format slug@hostname where the hostname is slugified from backendName in .tenex/config.json (e.g., "pablos laptop" → "pablos-laptop") to avoid ambiguity for agent-to-agent messaging; the raw value is preserved in storage and Nostr events, and the project is shown as a separate secondary field. Project-scoped agent lookup resolves a bare agent name (e.g., `codex`) as `codex@$currentProject` and does not fall back through a global profile before project presence. who defaults to showing only agents in the current project (resolved from cwd) and includes a footer listing other projects with their agent counts and one-liner descriptions; project descriptions come from NIP-29 kind 39000 group metadata events (the about tag), left empty if no metadata exists. tenex-edge who --project $slug shows agents in the specified project with other projects in the footer; tenex-edge who --all-projects shows every agent across all projects flat with no footer, and the project column is visible for each row. The live view (who --live) uses the same colorized output as the plain who command and appends a dim footer line with refresh interval and quit instructions; the tabular plain-text renderer is removed. Q1 collision logging (agent, path, timestamp) starts day one and lives in the tenex-edge substrate as awareness data, not in any host adapter.

<!-- citations: [^56f9f-5] [^56f9f-10] [^da7ab-1] [^f3a73-118] [^240ff-11] [^98f99-31] [^56f9f-18] [^rollo-6] [^rollo-20] [^1b868-50] -->

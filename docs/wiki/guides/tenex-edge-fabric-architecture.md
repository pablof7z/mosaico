---
title: tenex-edge Fabric Architecture
slug: tenex-edge-fabric-architecture
topic: tenex-edge
summary: "The domain speaks in two concern-planes: Project-State (open_project, roster, presence, status, project_meta) and Communications (send, inbox)"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-16
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:40a4d401-2520-4781-b747-b0ef19594bed
  - session:0bc06206-1f30-4e35-8373-f31d0f5c1dcc
  - session:a88513d3-754f-4369-b440-72c8d29331e2
  - session:rollout-2026-06-09T15-35-48-019eac61-c1bb-7391-b237-7378101f099a
---

# tenex-edge Fabric Architecture

## Domain Events

The domain speaks in two concern-planes: Project-State (open_project, roster, presence, status, project_meta) and Communications (send, inbox), with ACL as a shared is_member predicate both planes consult rather than a third plane. The five DomainEvent variants are Profile, Presence, Activity, Status, and Mention — these are fixed and non-negotiable; a new codec must supply a wire mapping for each but cannot change the domain taxonomy. <!-- [^d208c-1] -->

A Mention message must carry the sender's session id (from_session) as a return envelope, so the recipient knows which session to reply to. The author pubkey alone is insufficient because sibling sessions share it. <!-- [^d208c-2] -->

The injected mention format includes a reply-to handle: `[mention from <slug>@<project> · reply-to <session_id>] <body>`, falling back to `slug@project` when the session is not resolvable. <!-- [^d208c-3] -->

## FabricProvider

A FabricProvider bundles four single-responsibility capabilities: Lifecycle reactor (side-effects of domain events), Materializer (composes Wire codec + Delivery, owns admit + derive + upsert), Wire codec (pure DomainEvent ⇄ envelope), and Delivery (publish + subscribe, owns REQ-filters / gossip / MLS-stream privately). The Fabric trait replaces the Codec trait with publish/subscribe methods that traffic in DomainEvent and SubScope, making encode/decode/filters/EventBuilder private implementation details of a NostrFabric, and enabling non-Nostr transports (MLS-native, gRPC) as sibling Fabric implementations. The turn-start fabric injection (push_turn_fabric_block) uses the agent renderer (render_who_agent) with a one-line lead-in. The Materializer capability explicitly composes the Wire codec and Delivery capabilities rather than re-owning decode and subscribe, preserving single-responsibility attribution. When a new project spins up, the lifecycle reactor side-effect varies by fabric: NIP-29 creates a closed group and adds the agent as a member; MLS creates a group and invites the agent; kind1 performs no group creation (groups are simply t-tags). All wire-shape construction above the fabric provider seam must be closed in the current task, not deferred as a follow-up. The two divergent propose implementations are consolidated into one: fabric's version with master's no-live-session fallback grafted in (the canonical-thread dual-write was later removed). The session engine takes the provider instead of codec+transport.

Sending a message is optimistic but reconciled: sign/publish, insert a local message row with sync_state, then advance sync_state to accepted/echoed/failed as the relay or fabric confirms. <!-- [^rollo-28] -->

The Provider/store seam design constrains Delivery to stream raw envelopes, while Materializer owns decode, ACL, derivation, and store writes. <!-- [^rollo-30] -->

The FabricProvider trait introduction uses a concrete provider or enum first, and only boxes it later with an object-safe/boxed-future shape if multiple providers need runtime dispatch, avoiding an invalid async dyn trait. <!-- [^rollo-31] -->

<!-- citations: [^d208c-4] [^d208c-5] [^d208c-6] [^0bc06-3] [^0bc06-4] [^0bc06-6] [^a8851-1] -->
## Store and Read Interface

All data is read from a unified local store (state.db); how data is hydrated is invisible to every reader. The provider is a write-side materializer that decodes → ACL-admits → derives → upserts canonical rows. Reads are SELECTs against the store, never through the provider. Whether the fabric is kind:1, NIP-29, MLS, A2A, or any other protocol is adapter-facing and irrelevant to the read interface.

The canonical entity set read from the store includes: the list of projects and their metadata, and the list of agents inside a project with any other metadata.

The canonical messages table must carry author_session (the return envelope) alongside author_pubkey, and the inbox → messages migration must copy inbox.from_session to messages.author_session so the return envelope is never dropped.

Legacy tables (inbox, project_meta, agent_status, peer_sessions, sessions) are deliberately retained as the authoritative readers per the architecture doc's §6 escape hatch; the canonical read model is written but largely unread until a future cutover.

Project identifiers are local canonical IDs; native identifiers (h tags, NIP-29 group ids, MLS conversation ids) are stored as origins, not treated as universal truth. <!-- [^rollo-27] -->

Inbound events that arrive before membership is hydrated go to inbound_quarantine instead of being dropped, providing temporal ACL semantics. <!-- [^rollo-29] -->

<!-- citations: [^d208c-7] [^d208c-8] [^d208c-9] [^ab999-15] -->
## Access Control

The is_member ACL gate is consulted twice over the same store rows: once as a write-side admission predicate during materialization, and once as a read-side query, never on the wire. <!-- [^d208c-10] -->

## Provenance and Discovery

Project metadata provenance varies by fabric: NIP-29 uses relay-authored kind:39000 (canonical and shared), MLS uses member-authored group-context (cryptographically scoped), and kind1 has no native carrier (list is derived from observed tags + local dirs, description is Option/local and may diverge per machine). Agent enumeration follows the same pattern as project enumeration (uniform shape, provider-owned source, derived/Option fallback for kind1), but agent metadata is uniformly self-sovereign — the agent signs its own profile on every fabric — so the per-fabric axis that matters is discovery scope and owner-claim authorization, not metadata provenance. <!-- [^d208c-11] -->

## NIP-29 Fabric Specifics

NIP-29 group management (group_create, group_lock_closed, group_put_user, kinds 9007/9002/9000, and relay-authored group-state subscriptions 39000/39001/39002) is an access-control/addressing concern that belongs to the NIP-29 fabric, not to the domain event codec or the kind1 event-shaping layer. <!-- [^d208c-12] -->

## Wire Codec Rules

The kind1 codec collapses Activity and Mention onto kind:1 and splits them on decode by the presence of an agent tag plus a p-tag (kind1.rs:276-298). Any codec that reuses a kind across two domain events must define its own disambiguation rule. kind:1 user-prompt events must include a session-id tag so they are routed only to the originating session, not fanned out to all sessions of the agent. The agent's own user prompt must be pre-marked as delivered (suppress_inbox_event) so it never appears as an unread inbox item when relay-echoed back.

<!-- citations: [^d208c-13] [^40a4d-1] -->

## Documentation

A fabric-architecture overview document exists as a separate one-page version (`docs/fabric-architecture-overview.md`) alongside the detailed version, keeping only load-bearing ideas and dropping schema, capability tables, phase plans, and accessors. <!-- [^d208c-15] -->

## Implementation Status

The fabric architecture refactor was implemented in a git worktree at /Users/pablofernandez/src/tenex-edge-fabric on branch fabric-architecture, with all 9 phases (0–8) completed and committed. The tenex-edge-fabric worktree is fully merged and can be pruned.

Phase 0 added 20 freeze tests pinning existing behavior (routing/dedup/ACL/context/idempotency) before any structural changes, and all tests stayed green throughout all phases.

Phase 1 added 4 canonical tables (projects, project_origins, inbound_quarantine, membership), gen_id(), and an idempotent backfill function. The threads/thread_origins/messages/message_recipients tables were later removed when the kind:1 thread plane was dropped.

Phase 2 added read-model methods and write-facing materializer methods to Store, rewired readers in cli.rs and server.rs to use read-model methods, and kept drain_inbox as a delivery write (not a read).

Phase 3 extracted Nostr delivery into src/fabric/* (RawEnvelope, Scope, NostrDelivery, Kind1WireCodec), moved filter construction to scope_filters(), and left the filters_cover_all_kinds_and_mentions test unmodified as the equivalence oracle.

Phase 4 turned handle_incoming into a thin dispatch to fabric::materialize, with Kind1Materializer and Nip29Materializer handling the actual work; quarantine and ACL membership-gating were explicitly deferred to avoid breaking freeze tests.

Phase 5 introduced Kind1Nip29Provider bundling delivery/codec/materializer/lifecycle behind one provider, rewired DaemonState's four entry points, and preserved the single-writer invariant via shared Arc<Mutex<Store>>.

Phase 6 introduced SendIntent/OutboundReceipt, provider.send(), sync_state tracking, and dual-write of canonical messages/message_recipients for both outbound and inbound; the legacy inbox path remains authoritative and frozen.

Phase 7 added thread/message reads (list_threads, messages_for_thread, thread_meta as RPC+CLI) and NIP-10 root e-tag reply threading; both were later removed when the kind:1 thread plane was dropped.

Phase 8 removed Codec::filters and SubScope from the seam, moved group builders into fabric/nip29/lifecycle.rs, wired backfill at daemon startup, moved rpc_project_list's 39000 parsing into the provider, and replaced TODO(phase 8) markers with 'Retained storage' notes rather than doing the risky inbox-over-messages reader swap.

Startup backfill on a populated database was verified to work cleanly: after seeding legacy rows and restarting, the daemon migrated projects=1, project_origins=1, membership=2 with about and roles preserved.

Live e2e testing with real claude, codex, and opencode agents validated the fabric provider seam closure.

The implementation plan is documented in-place at docs/fabric-architecture.md and includes: guardrails for preserving current host/RPC behavior, Phase 0 regression tests before moving code, canonical read-model schema and backfill rules, StoreReader/StoreWriter split, raw Nostr delivery extraction from Codec::filters, materializer extraction from server.rs::handle_incoming, FabricProvider introduction, outbound message sync_state cutover, and legacy cleanup and validation ladder. <!-- [^rollo-32] -->

<!-- citations: [^ab999-3] [^ab999-4] [^ab999-5] [^ab999-6] [^ab999-7] [^ab999-8] [^ab999-9] [^ab999-10] [^ab999-11] [^ab999-12] [^ab999-13] [^ab999-14] [^0bc06-1] [^0bc06-2] [^0bc06-7] -->

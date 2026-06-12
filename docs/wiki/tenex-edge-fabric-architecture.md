---
title: Tenex-Edge Fabric Architecture
slug: tenex-edge-fabric-architecture
topic: tenex-edge
summary: "A FabricProvider bundles four single-responsibility capabilities: Lifecycle reactor (project spin-up side-effects), Membership source (hydrates and streams the"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-12
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
  - session:36cc4546-228e-4d07-a1a8-9d0cd7cd5a6c
  - session:0bc06206-1f30-4e35-8373-f31d0f5c1dcc
---

# Tenex-Edge Fabric Architecture

## Fabric Provider Capabilities

A FabricProvider (replacing the former Codec trait) bundles four single-responsibility capabilities: Lifecycle reactor (side-effects of project events), Materializer (composes Wire-codec + Delivery, owning only admit, derive, and upsert — it does not re-own decode or subscribe), Wire codec (pure DomainEvent ⇄ envelope), and Delivery (publish + subscribe-for-scope, owning transport-specific details like REQ-filters internally). The Fabric trait defines publish and subscribe methods operating on DomainEvent and SubScope (abstract types), making nostr_sdk types private implementation details of a NostrFabric. Whether the system uses kind:1, NIP-29, MLS, A2A, or any other protocol is purely an adapter/fabric-facing concern and is irrelevant to the use of the data; it must not leak into the read interface. The fabric/provider is a write-side materializer that decodes, ACL-admits, derives, and upserts canonical rows into the unified local store (state.db), not a read-through query path; all reads come from one unified local store where how data was hydrated is invisible to every reader (CQRS-style split). A future Fabric trait refactor to decouple from nostr_sdk types will be a self-contained module refactor with no domain layer changes.

The fabric-architecture branch is complete, working, and well-tested for its defined scope, executing the strangler-pattern migration through Phase 8 with dual-writing of legacy tables alongside the canonical store. No migration or backwards compatibility is required for adopting the fabric architecture; the new code must simply be verified to work end-to-end. <!-- [^0bc06-1] -->

All wire-shape leaks above the provider seam must be closed as part of this task, not deferred as follow-up work. Six leaks currently bypass the provider: rpc_turn_end publishes TurnReply via Kind1Codec and transport directly; rpc_project_edit builds inline kind:9002 events (NIP-29 lifecycle that belongs in fabric/nip29/lifecycle.rs); doctor builds an inline kind:1 probe (belongs in the delivery layer); runtime.rs session engine publishes Presence/Status/Activity via Kind1Codec directly. The seam-closing work must be a single dedicated commit that moves all six leaks behind the fabric layer, placed after the mechanical rebase and before the tail bug fixes. Each rebased commit must preserve original behavior so it stays auditable against its original, even if it temporarily adds instances of an existing known leak. <!-- [^0bc06-2] -->

<!-- citations: [^d208c-3] [^36cc4-1] [^36cc4-2] [^d208c-10] [^d208c-17] [^d208c-25] [^d208c-30] -->
## Project Spin-Up Side-Effects

When a new project spins up in a directory that has never been run before, the active fabric triggers side-effects: NIP-29 fabric creates a group and adds the agent as a member; MLS fabric creates a group and sends an invite; kind1 fabric performs no group-creation side-effect (groups are simply 't' tags, presence events carry the 't' tag, and membership is determined by a local whitelist of known/accepted pubkeys).

<!-- citations: [^d208c-4] [^d208c-11] [^d208c-18] -->
## ACL as a Shared Predicate

Access control (e.g., who can p-tag an agent with a message) is hydrated by the active fabric's membership model (NIP-29 membership, MLS roster, or kind1 whitelist of accepted/known pubkeys). ACL is not a third plane but an `is_member?` predicate that both Project-State and Communications planes consult. The `is_member` gate is consulted twice over the same store rows: once as a write-side admission predicate during materialization, and once as a read-side query, but never on the wire. The `is_member` gate must live in the domain because it can never be skipped, even when enforcement occurs server-side (e.g., NIP-29), cryptographically (e.g., MLS), or client-side (e.g., kind1).

<!-- citations: [^d208c-5] [^d208c-12] [^d208c-19] [^d208c-26] [^d208c-33] -->
## Roster and ACL Unification

Roster and ACL are a single source viewed two ways, not two separate sources of truth. <!-- [^d208c-6] -->

## NIP-29 as an Access-Control Concern

NIP-29 group management is an access-control and addressing concern orthogonal to event wire-shaping, and should be a property of a nostr transport/ACL strategy rather than a property of a kind1 event codec. <!-- [^d208c-7] -->

## Concern Planes

The architecture must have very clear scoping of concerns / Single Responsibility Principle. The domain verbs are organized into two planes: Project-State (open_project, list_projects, roster, presence, status, project_meta) and Communications (send, inbox, threads, thread_meta), with an ACL (`is_member?`) predicate consulted by both planes.

Read paths query the canonical store and never reference a kind, tag, group, or relay directly. <!-- [^0bc06-3] -->

<!-- citations: [^d208c-13] [^d208c-20] [^d208c-31] -->
## Project Metadata as a Provider Capability

ProjectMeta must be modeled as a provider-owned source capability, exposed as a queryable and streamable pair (`query_once`, `subscribe_changes`), identical in shape to how roster/membership works. Project list and metadata provenance varies per fabric: NIP-29 uses relay-authored kind:39000 (canonical & shared); MLS uses member-authored group-context (cryptographically scoped); kind1 has no native carrier (list is derived from observed tags + local dirs, description is Option/local and may diverge). The domain must accept that for non-authoritative fabrics like kind1, the project list is derived rather than enumerated, and description is `Option<String>` to accommodate non-authoritative fabrics where metadata is client-local and can diverge. Project enumeration and metadata retrieval support both pull (`query_once`) and live (`subscribe to changes`) modes, owned by the provider.

<!-- citations: [^d208c-14] [^d208c-21] -->

## Agent List Enumeration

Agent list enumeration follows the same pattern as project list: uniform shape, provider-owned source, with derived/Option fallback for kind1 (derived from kind:0 profiles + group membership, not from observed `agent`-tagged authors). <!-- [^d208c-22] -->

## Read-Model Entities

The read-model entities in the unified local store are: projects and their metadata, agents inside a project and their metadata, threads inside a project, messages inside a thread, and the recipient of each thread or message. Threading is resolved as a store noun that the materializer derives, not an open question.

<!-- citations: [^d208c-27] [^d208c-32] -->
## Domain Verb Split

Domain verbs split into reads and intents: reads query the unified store with no provider in the call path; intents are the only verbs that touch a provider. <!-- [^d208c-28] -->

## Publication

The architecture document is published as a NIP-23 (kind:30023) event on nos.lol under the d-tag `tenex-edge-fabric-architecture`. <!-- [^d208c-29] -->

## Integration and Rebase Sequence

Master must be rebased onto the fabric-architecture branch first, then the three tail bugs must be fixed and verified. In-flight statusline work must be committed before starting the rebase so the tree is clean. <!-- [^0bc06-4] -->

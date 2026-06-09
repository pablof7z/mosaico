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
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
  - session:36cc4546-228e-4d07-a1a8-9d0cd7cd5a6c
---

# Tenex-Edge Fabric Architecture

## Fabric Provider Capabilities

A FabricProvider bundles four single-responsibility capabilities: Lifecycle reactor (project spin-up side-effects), Membership source (hydrates and streams the ACL), Wire codec (DomainEvent ⇄ envelope), and Delivery (publish and subscribe-for-scope). The Fabric trait must sit at the DomainEvent/SubScope level so that transport implementations (e.g., NostrFabric, MLS-native, gRPC) become siblings, not nested nostr-specific codec implementations. A future Fabric trait refactor to decouple from nostr_sdk types will be a self-contained module refactor with no domain layer changes.

<!-- citations: [^d208c-3] [^36cc4-1] [^36cc4-2] [^d208c-10] -->
## Project Spin-Up Side-Effects

When a new project spins up, the active fabric determines the side-effects: NIP-29 fabric creates a group and adds the agent as a member; MLS fabric creates a group and sends an invite; kind1 fabric performs no group-creation side-effect. <!-- [^d208c-4] -->


When a new project spins up, the active fabric determines the side-effects: NIP-29 fabric creates a group and adds the agent as a member; MLS fabric creates a group and sends an invite; kind1 fabric performs no group-creation side-effect. In the kind1 codec, no new group is created on project spin-up; groups are simply `t` tags, presence events carry the `t` tag, and membership is determined by a local whitelist of known/accepted pubkeys. <!-- [^d208c-11] -->
## ACL as a Shared Predicate

ACL is not a third plane but an `is_member?` predicate that both Project-State and Communications planes consult. The `is_member` gate must live in the domain because it can never be skipped, even when enforcement occurs server-side (e.g., NIP-29) or cryptographically (e.g., MLS). <!-- [^d208c-5] -->


ACL is not a third plane but an `is_member?` predicate that both Project-State and Communications planes consult. The `is_member` gate must live in the domain because it can never be skipped, even when enforcement occurs server-side (e.g., NIP-29), cryptographically (e.g., MLS), or client-side (e.g., kind1). <!-- [^d208c-12] -->
## Roster and ACL Unification

Roster and ACL are a single source viewed two ways, not two separate sources of truth. <!-- [^d208c-6] -->

## NIP-29 as an Access-Control Concern

NIP-29 group management is an access-control and addressing concern orthogonal to event wire-shaping, and should be a property of a nostr transport/ACL strategy rather than a property of a kind1 event codec. <!-- [^d208c-7] -->

## Concern Planes

The verbs of the system must be organized into two concern-planes: Project-State (open_project, roster, presence, status, project_meta, list_projects) and Communications (send, inbox, threads, thread_meta). <!-- [^d208c-13] -->

## Project Metadata as a Provider Capability

ProjectMeta must be modeled as a provider-owned source capability, exposed as a queryable and streamable pair (`query_once`, `subscribe_changes`), identical in shape to how roster/membership works. Project descriptions must be modeled as `Option<String>` to accommodate non-authoritative fabrics (like kind1) where metadata is client-local and can eventually diverge. <!-- [^d208c-14] -->

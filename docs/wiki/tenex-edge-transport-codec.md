---
title: Tenex-Edge Transport Codec
slug: tenex-edge-transport-codec
topic: tenex-edge
summary: Envelope encoding and decoding is modularized as a codec set providing per-event encode, decode, and subscribe operations
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-08
updated: 2026-06-09
verified: 2026-06-08
compiled-from: conversation
sources:
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
  - session:36cc4546-228e-4d07-a1a8-9d0cd7cd5a6c
---

# Tenex-Edge Transport Codec

## Codec Set Architecture

Envelope encoding and decoding is modularized as a codec set providing per-event encode, decode, and subscribe operations, decoupling envelope shapes from business logic so that alternative transports (such as another NIP-29 relay policy or Marmot/MLS) can be added as additional codec adapters without modifying domain logic. The codec set is granular per event type rather than a monolith. Heartbeat activity is included in the codec set because its shape varies across different codecs. The initial shape adapter uses kind:1 notes and kind:30315 heartbeats/statuses, all project-scoped with the NIP-29 `h` tag.

The `Codec` trait defines four verbs: `name` (stable wire-shape identifier), `encode` (domain to unsigned wire envelope), `decode` (wire to domain, or None if unrecognized), and `filters` (subscriptions fetching everything the codec speaks). A codec's `filters` verb must produce subscriptions covering every event type its `decode` can recognize. A new codec inherits a fixed domain taxonomy of five nouns (Profile, Presence, Activity, Status, Mention) and must supply a wire mapping for them. Any codec that reuses a kind across two domain events must define its own tie-breaker disambiguation rules.

Domain code must never name a specific Nostr kind, tag, or wire-protocol concept, keeping the domain layer transport-agnostic. The domain layer (`domain.rs` and `SubScope`) is genuinely transport-agnostic and names no kind, tag, or relay, exposing only the five nouns (Profile, Presence, Activity, Status, Mention).

The initial Nostr codec maps these nouns as follows: Profile → kind:0 with content `{"name": slug}` and tags `["host", host]` and `["p", owner]`; Presence → kind:30315 (NIP-38) with tags `["d", "tenex-edge-presence:<session>"], ["h", project], ["session-id", id], ["agent", pk, slug], ["p", audience]`; Activity → kind:1 with tags `["h", project], ["agent", pk, slug]`; Status → kind:30315 with tags `["h", project], ["d", project], ["agent", pk, slug]`; Mention → kind:1 with tags `["p", to_pubkey], ["h", project], ["agent", pk, slug]`. Activity and Mention both use kind:1 and are disambiguated on decode by the presence of a `p` tag. Profile, Presence, and Mention builders use `.allow_self_tagging()` to prevent nostr-sdk from stripping `p` tags equal to the author.

NIP-42 AUTH must be built into the transport layer from day one, since relays almost certainly require it for publishes and silently reject publishes without it. Additionally, transport must force NIP-42 AUTH completion (a warm-up fetch) before any subscribe, because relay.tenex.chat requires auth for reads and closes subscriptions opened before auth completes.

NIP-29 group management and group-state subscriptions should be properties of a nostr transport/ACL strategy rather than fused into the kind1 event codec.

Transport is built on the lean `nostr`/`nostr-sdk` stack behind a Transport trait, rather than embedding the full NMP kernel (which is unsuitable for headless CLI daemons), with NMP documented as the intended future swap-in behind the codec seam, not at the transport layer. However, the current `Codec` trait is coupled to nostr because its `encode`, `decode`, and `filters` signatures use `nostr_sdk` types (`EventBuilder`, `Event`, `Filter`), meaning it can only swap NIPs, not underlying transports. Furthermore, the `filters` verb bakes in relay REQ semantics, making it incompatible with push/gossip transports that use a different fetch model. To support any transport, the swap seam should be elevated to a `Fabric` trait that takes abstract `DomainEvent` and `SubScope` types, making `encode`/`decode`/`filters` private implementation details of a `NostrFabric`.

The runtime processes both deduped Event notifications and raw RelayPoolNotification::Message{Event} variants to avoid silent pending-discovery failures.

Codec integration is feature-complete for M1 with no immediate work needed.

<!-- citations: [^f3a73-36] [^f3a73-37] [^f3a73-38] [^f3a73-46] [^f3a73-58] [^f3a73-79] [^f3a73-85] [^f3a73-97] [^d208c-1] [^36cc4-6] -->

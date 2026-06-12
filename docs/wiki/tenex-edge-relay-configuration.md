---
title: Tenex-Edge Relay Configuration
slug: tenex-edge-relay-configuration
topic: tenex-edge
summary: "The default relay is wss://nip29.f7z.io (using nip29)"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:f9bdcf4c-c972-46ff-91b8-9e30785d3331
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:36cc4546-228e-4d07-a1a8-9d0cd7cd5a6c
  - session:ab9998c4-6e65-410e-b298-122a2072171c
---

# Tenex-Edge Relay Configuration

## Default Relay

Presence, activity, status, and mention events all use an h tag with the project slug as a namespace filter, but the relay they connect to (relay.tenex.chat) is a standard Nostr relay with auth, not a NIP-29 relay enforcing group semantics. The relay is the remote bridge between phone and daemon; push delivery to a backgrounded phone (via APNs/FCM) is deferred as unsolved. The refactored tenex-edge binary and the tenex-off app must share the same relay to close the loop; a relay mismatch means the app can't see proposals and comments can't reach agents. Transport::connect forces NIP-42 AUTH completion with a fetch_events warm-up before any subscribe, because relay.tenex.chat requires AUTH for reads and subscriptions opened pre-auth are closed. The compiled-in DEFAULT_RELAY in src/config.rs is wss://nip29.f7z.io. The live config at ~/.tenex/config.json explicitly specifies the relays array (including wss://nip29.f7z.io) rather than relying on the compiled-in default. A signed event published over a differently-authenticated relay connection lands under the signing key's authorship (publish_signed method). The project list and project edit target nip29.f7z.io, a proper NIP-29 relay that tracks kind:39000 group metadata and expects kind:9002 for updates. Publishing a proposal to a closed, auth-gated NIP-29 relay such as nip29.f7z.io fails for unauthorized or unknown agent keys, whereas open relays such as relay.primal.net accept them but may exhibit ingestion lag. The live external nip29_probe test (wss://nip29.f7z.io) is not run as a regression oracle because it probes relay rules, not this codebase's behavior.

<!-- citations: [^ab999-55] [^f9bdc-2] [^98f99-17] [^98f99-22] [^36cc4-5] [^ab999-40] [^ab999-62] -->

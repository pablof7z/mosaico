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
updated: 2026-06-12
verified: 2026-06-10
compiled-from: conversation
sources:
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
  - session:da7ab617-89fb-4b68-9e2d-3f251fe6c1d9
---

# Tenex-Edge Presence

## Tenex Edge Presence

tenex-edge does not publish 24010/24011 events; received 24011 presence events are ignored, not emitted. The live agent indicator in tenex-edge tail displays a DomainEvent::Presence derived from kind 30315 (a NIP-38-style addressable heartbeat event keyed by d = "tenex-edge-presence:<session>" with an expiration tag). Remote agents in the `who` command display their actual hostname (e.g., `(tower)`) instead of the generic `(remote)` string.

<!-- citations: [^56f9f-5] [^56f9f-10] [^da7ab-1] -->

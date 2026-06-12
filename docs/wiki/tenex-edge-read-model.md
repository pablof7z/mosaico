---
title: Tenex-Edge Read Model
slug: tenex-edge-read-model
topic: tenex-edge
summary: The read model is the contract; the provider is a write-side materializer
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
  - session:ab9998c4-6e65-410e-b298-122a2072171c
---

# Tenex-Edge Read Model

## Read Model Contract

The read model is the contract; the provider is a write-side materializer. How data was hydrated is invisible to every reader. All data reads come from a unified local store (state.db); how the data was hydrated (which fabric, which protocol) is completely invisible to readers. Reads query the store; only intents touch a provider. In practice, the canonical read model is written but largely unread — inbox, who, and turn-start stay on legacy tables by deliberate retention per the architecture doc's escape hatch, rather than doing a full read-model cutover. Phase 2 splits StoreReader/StoreWriter so readers go through read-model methods, while drain_inbox stays in turn-start as a delivery write rather than a read. Phase 6 dual-writes canonical messages/message_recipients for both inbound and outbound, while the legacy inbox stays the authoritative reader and route_mention_into stays frozen. Canonical writes in Phase 6 are best-effort (.ok(), never failing the legacy path).

<!-- citations: [^d208c-44] [^d208c-45] [^d208c-49] [^ab999-11] [^ab999-39] -->

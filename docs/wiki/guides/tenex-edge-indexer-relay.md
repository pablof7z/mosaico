---
title: tenex-edge Indexer Relay
slug: tenex-edge-indexer-relay
topic: tenex-edge
summary: "tenex-edge publishes kind:0 events to a configurable indexer relay, defaulting to wss://purplepag.es"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-14
updated: 2026-06-28
verified: 2026-06-14
compiled-from: conversation
sources:
  - session:ab43967d-95d5-49fd-aaf6-4bc65d80774e
  - session:b07a57a3-67a1-4c44-a8fc-58a1bb97860a
---

# tenex-edge Indexer Relay

## Indexer Relay Configuration

tenex-edge publishes kind:0 events to a configurable indexer relay, defaulting to wss://purplepag.es. The indexer relay is also checked/queried for kind:0 info (e.g. for tenex-edge who resolution). The indexer relay is configurable in the config file via an `indexerRelay` field. <!-- [^ab439-1] -->

## Relay Connection and Identity

At daemon startup, the transport connects to the combined list of `cfg.relays` and `cfg.indexer_relay` (deduped). The `provider_instance` hash is computed from only `cfg.relays` (excluding the indexer relay), preserving canonical IDs. <!-- [^ab439-2] -->

When tenex-edge starts, the configured relay is displayed. <!-- [^b07a5-b6232] -->

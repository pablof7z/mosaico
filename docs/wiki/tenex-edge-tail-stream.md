---
title: Tenex-Edge Tail Stream
slug: tenex-edge-tail-stream
topic: tenex-edge
summary: The canonical store deduplicates writes on event id, but the tail v2 broadcast emits duplicate messages because one message produces identical tail events for e
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-12
updated: 2026-06-12
verified: 2026-06-12
compiled-from: conversation
sources:
  - session:0bc06206-1f30-4e35-8373-f31d0f5c1dcc
---

# Tenex-Edge Tail Stream

## Tail Stream Bugs

The canonical store deduplicates writes on event id, but the tail v2 broadcast emits duplicate messages because one message produces identical tail events for every matching subscription. The inbound tail stream renders an empty sender slug because the server emits m.from.slug directly from the wire instead of resolving by pubkey from the store like the outbound and backfill paths do. The inbound tail stream attributes new root messages to the wrong thread because it guesses via latest_thread_for_inbound heuristic instead of using the actual thread id from materialization. <!-- [^0bc06-5] -->

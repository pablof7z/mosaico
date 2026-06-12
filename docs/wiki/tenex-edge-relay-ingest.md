---
title: Tenex-Edge Relay Ingest
slug: tenex-edge-relay-ingest
topic: tenex-edge
summary: The `handle_incoming` function deduplicates relay events by event ID using a 512-slot ring buffer (`seen_events`) in `DaemonState` to prevent fanout duplication
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-10
updated: 2026-06-10
verified: 2026-06-10
compiled-from: conversation
sources:
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
---

# Tenex-Edge Relay Ingest

## Event Deduplication

The `handle_incoming` function deduplicates relay events by event ID using a 512-slot ring buffer (`seen_events`) in `DaemonState` to prevent fanout duplication. <!-- [^56f9f-7] -->

---
title: Tenex-Edge Daemon Write-Disabled Error
slug: tenex-edge-daemon-write-disabled
topic: tenex-edge
summary: "The \\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\"write actions are disabled\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\" error message originates client-side from the daemon's own `nostr-relay-pool` Rust dependency (`Error::WriteDisabled`), not fro"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-03
verified: 2026-07-03
compiled-from: conversation
sources:
  - session:a685f611-39bd-4a18-a6b7-ea4e38334b82
---

# Tenex-Edge Daemon Write-Disabled Error

## Origin of the "Write actions are disabled" Error

The "write actions are disabled" error message originates client-side from the daemon's own `nostr-relay-pool` Rust dependency (`Error::WriteDisabled`), not from the relay server. <!-- [^a685f-fa9c7] -->

The `Error::WriteDisabled` error fires when the daemon's in-memory `Relay` object has its local `RelayServiceFlags` missing the `WRITE` flag, causing the SDK to refuse to put the `EVENT` on the wire before it ever reaches the relay. <!-- [^a685f-fa9c7] -->

The daemon's `transport.rs` wraps the `WriteDisabled` error as `"relay rejected event: {msg}"`, which is misleading because nothing was actually rejected by the relay — the daemon never sent it. <!-- [^a685f-51c9c] -->

## Intermittent Startup Failures

Intermittent `WriteDisabled` failures during daemon startup are consistent with a startup race in the relay-pool's internal state when ~10 reconciled sessions all fire a domain-event publish through the single shared `Transport` simultaneously, rather than a permanently-missing flag. <!-- [^a685f-69259] -->

Session spawn proceeds regardless of domain-event publish failures — the daemon derives the session key and spawns the session engine after each error. <!-- [^a685f-fbb3e] -->

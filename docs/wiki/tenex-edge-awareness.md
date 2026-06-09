---
title: Tenex-Edge Awareness
slug: tenex-edge-awareness
topic: tenex-edge
summary: The awareness board's state model lives behind a transport interface, so that switching from local storage to network sync is a transport swap rather than a rew
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
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
---

# Tenex-Edge Awareness

## Transport Architecture

The board's state model lives behind a transport interface so that lifting the same state onto the network fabric is a transport swap rather than a rewrite. Envelope shapes are decoupled from business logic via a modularized encoder/decoder (codec set), allowing alternative transports (e.g., NIP-29, Marmot/MLS) to be added as separate shape adapters without modifying domain logic. Rung 0 transport uses local SQLite; Rung 1 swaps to the proven Nostr kernel.

<!-- citations: [^f3a73-4] [^f3a73-8] [^f3a73-17] -->
## Collision Logging

Q1 collision logging lives entirely inside tenex-edge's awareness model as the substrate observing activity reported across its boundary, independent of any specific host. The passive collision logger records (agent, path, timestamp) with no coordination logic to gather data for Rung 2 decisions. <!-- [^f3a73-9] -->

## Agent Activity Events

Agent activity is published as Nostr kind 1 events with a NIP-29 `h` tag whose value is the project slug, plus human-readable intent-level content. Activity distillation is auto-distilled (not agent-manual), using an LLM step to produce intent-level awareness lines. Agents maintain a running NIP-38 status per project, `h`-tagging the project slug and retaining a `d` value for replacement identity, with the status empty when idle. NIP-38 status events include a NIP-40 expiration tag so that stale status clears even if the process dies unexpectedly.

The cwd field is added to the status event so that the `who` renderer can display working directories. The project-relative form of cwd is put on the wire (not absolute $HOME paths) to mitigate leaking filesystem paths in world-readable public events on relay.tenex.chat. <!-- [^162f9-10] -->

<!-- citations: [^f3a73-18] [^f3a73-23] -->

## Presence Events

Presence is published every 30 seconds as an expiring `kind:30315` heartbeat with `h` set to the project slug, `d` set to `tenex-edge-presence:<session-id>`, `agent` carrying agent pubkey and slug, `session-id` carrying the host session ID, and `expiration` bounding liveness. <!-- [^f3a73-24] -->

## Tail Client

A `tenex-edge tail -f <optional-project-slug>` command provides a colorized streaming client of all awareness activity. <!-- [^f3a73-25] -->

## Legacy Migration

Once tenex-edge's awareness board is live, pc's legacy awareness module will be removed and pc will become a thin adapter that injects context and captures host events, consuming awareness deltas from tenex-edge instead.

<!-- citations: [^f3a73-26] [^f3a73-87] -->

## `who` Command

The `who` command displays agents as: agent@hostname [session $id] [$relativePwd] followed by their current status, where relativePwd is relative to the project root (showing `.` for the root). Agents on the same machine show no host annotation; agents on a different host are annotated with (remote). <!-- [^162f9-11] -->

---
title: Tenex-Edge Inbox Delivery
slug: tenex-edge-inbox-delivery
topic: tenex-edge
summary: Inbox delivery uses an atomic `UPDATE â¦ SET state='delivered' â¦ RETURNING` claim so the first drainer (pty paste or hook) wins and the other gets nothing
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:d39d3357-06d0-418a-bdbe-f288a9f9670f
---

# Tenex-Edge Inbox Delivery

## Atomic Claim-and-Deliver

Inbox delivery uses an atomic `UPDATE … SET state='delivered' … RETURNING` claim so the first drainer (pty paste or hook) wins and the other gets nothing. Atomicity is the dedup — there is no separate gate. <!-- [^d39d3-6ca4c] -->

## PTY Paste Failure Re-enqueue

PTY delivery re-enqueues a message to pending if the paste itself fails, so a dead pane doesn't silently eat a message. <!-- [^d39d3-94150] -->

## Hooks Path Context Rendering

The hooks path in `assemble_turn_start_context` renders only ambient context (skips the mention block) when the session has a live PTY session, because the terminal injection path owns direct-mention delivery. <!-- [^d39d3-b6337] -->

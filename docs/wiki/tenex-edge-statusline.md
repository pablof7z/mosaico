---
title: Tenex-Edge Statusline
slug: tenex-edge-statusline
topic: tenex-edge
summary: The statusline is a one-line awareness board representing the floor product (identity + awareness + passive collision signal) in the host terminal
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-12
updated: 2026-06-12
verified: 2026-06-12
compiled-from: conversation
sources:
  - session:e42f09d7-5fb0-438b-a356-216870390540
---

# Tenex-Edge Statusline

## Purpose

The statusline is a one-line awareness board representing the floor product (identity + awareness + passive collision signal) in the host terminal. It is the correct surface for the membership warning false-positive problem because, unlike injected context, it self-corrects as the cache refreshes. <!-- [^e42f0-1] -->

## Identity Segment

The statusline displays durable citizenship identity in the format `agent@host ⌗session_short_code`. <!-- [^e42f0-2] -->

## Membership Segment

The statusline persistently surfaces the NIP-29 membership warning when the session is not in the project group, showing the remediation command `tenex-edge project add`. When healthy and in the project group, the membership state is shown as `h:project ✓member`. <!-- [^e42f0-3] -->

## Roster Delta Segment

The statusline shows roster deltas (joins, leaves, status changes) between turns so fabric motion is visible between interactions. <!-- [^e42f0-4] -->

## Collision Watch Segment

The statusline provides a live collision watch signal when a sibling session is active in a file path touched by the current session. <!-- [^e42f0-5] -->

## Inbox Envelope Segment

The inbox envelope segment surfaces the newest sender and age, using the email-metaphor From/age format matching the envelope redesign. <!-- [^e42f0-6] -->

## Substrate Health Segment

The substrate health segment surfaces daemon (UDS), relay, heartbeat staleness, and db WAL state, reflecting the failure modes recorded in the docs. <!-- [^e42f0-7] -->

## Fleet Topology Segment

The fleet topology segment distinguishes local vs remote machines, since identity is machine-bound and the `who` command shows hostnames. <!-- [^e42f0-8] -->

## Provenance Trail Segment

The provenance trail segment shows the time since the last signed TurnReply event and whether the scrubber ran successfully. <!-- [^e42f0-9] -->

## ACL Attention Segment

The ACL attention segment surfaces pending foreign agents p-tagging the owner, directing the user to `tenex-edge acl` for allow/block decisions only a human can make. <!-- [^e42f0-10] -->

## Quiet Citizenship Layout

The quiet citizenship layout renders minimal output when all is well (e.g. `claude@kubrick ⌗601a36 ♥`) and becomes loud only on four attention states: no membership, inbox items, collision, or ACL pending. <!-- [^e42f0-11] -->

## Data Retrieval

The statusline data is retrieved via a single read from the daemon over the UDS JSON-RPC, using a new verb like `tenex-edge statusline --json` or reusing the read model behind `who` + `peek_inbox`. The statusline daemon verb must be pure-read with no state.db writes, like `peek_inbox` and turn-check, because Claude Code re-runs the statusline constantly and transient concurrent writers must not be reintroduced. <!-- [^e42f0-12] -->

## Degraded Mode

When the daemon is unreachable, the statusline degrades gracefully and displays a fail-open message like 'fabric blind, host unimpeded' rather than erroring. The statusline must fail open like the host adapters: if the daemon is down, render a degraded line or nothing, never blocking or erroring. <!-- [^e42f0-13] -->

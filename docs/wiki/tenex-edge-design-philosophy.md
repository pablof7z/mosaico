---
title: Tenex-Edge Design Philosophy
slug: tenex-edge-design-philosophy
topic: tenex-edge
summary: The design discussion operates at a higher, design-space level—what the thing is, what shape it should take, what is worth wanting, and where tensions and bets
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-07
updated: 2026-06-09
verified: 2026-06-07
compiled-from: conversation
sources:
  - session:8a3eb1b2-7bbf-4761-ad1a-411a0a1fa666
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
---

# Tenex-Edge Design Philosophy

## Design Space Focus

The design discussion operates at a higher, design-space level—what the thing is, what shape it should take, what is worth wanting, and where tensions and bets lie—rather than at the level of specifics such as event kinds or implementation mechanics.

tenex-edge owns identity and awareness as its own independent substrate, with no concept of any specific host (pc, Claude Code, etc.); the dependency arrow points one direction only from hosts to tenex-edge, never the reverse.

MCP is the natural shape of the tenex-edge public boundary — a standard interface that any external component integrates against with zero bilateral knowledge.

Cross-person agent communication is the north star, but it must be fenced off as a second phase with its own trust model; it should not leak into v1.

The Rung 0 MVP consists of identity, local awareness, and a passive collision logger that watches but does no coordination, with zero network, zero trust model, and zero consensus; agent routing is the next increment and is not part of v0.

Rung 1 lifts the same state onto the fabric (Nostr); this is a transport swap rather than a rewrite, provided the seam is clean.

tenex-edge's substrate is testable and valuable with no consumer attached; it can be driven from a test harness or CLI.

The implementation language is Rust (not NMP); NMP was found to be a full cross-platform app kernel unsuitable for a headless CLI daemon, so nostr-sdk is used behind the transport trait seam.

The architecture requires very clear scoping of concerns and adherence to Single Responsibility Principle (SRP).

The architectural proposal document is published as a kind:30023 (NIP-23 long-form) event on the nos.lol relay, signed by the user's `userNsec` identity, with the `d` tag `tenex-edge-fabric-architecture`.

<!-- citations: [^d208c-2] [^8a3eb-19] [^8a3eb-20] [^f3a73-10] [^f3a73-11] [^f3a73-12] [^f3a73-39] [^f3a73-50] [^f3a73-60] [^f3a73-81] [^f3a73-88] [^d208c-9] -->

---
title: Tenex-Off Publishing
slug: tenex-off-publishing
topic: tenex-edge
summary: "Tenex-off is a direct Nostr publisher: it signs and publishes kind:1 notes with the human's nsec and routing tags straight to relays, not via a send-message too"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-10
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:ab9998c4-6e65-410e-b298-122a2072171c
---

# Tenex-Off Publishing

## Direct Nostr Publisher

Tenex-off is a direct Nostr client that publishes kind:1 events signed with the owner's nsec straight to relays; it does not call a send-message tool or route through the daemon. A comment composed in the tenex-off Android app (anchored to a paragraph) and published as the app's verified event shape routes to the exact agent session that authored the proposal; Claude acted on such feedback by refactoring src/auth.rs. The app's deep-linked document view (OpenNaddr) captures the proposal's project/session routing tags, so comments published from that path carry the h/p/session-id tags needed for delivery. The tenex-off Rust core captures proposal routing tags (project/session) from the kind:30023, fetches proposals p-tagged to the owner, and stamps published notes with p/h/session-id routing tags so they reach the generating session. The tenex-off app has no relay-backed conversation feed or thread view yet — networking is behind the nmp_core observer model, and conversation rendering was achieved by seeding the app's local comment store with the real chatter events.

The product direction document is saved at tenex-off/Plans/product-direction.md, synthesized from a 5-opus brainstorm plus the user's four decisions. <!-- [^ab999-82] -->

<!-- citations: [^ab999-24] [^ab999-45] [^ab999-64] [^ab999-75] [^ab999-81] -->

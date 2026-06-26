---
type: episode-card
date: 2026-06-26
session: e07193a3-07de-4b5f-81f1-2623b055b20e
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/e07193a3-07de-4b5f-81f1-2623b055b20e.jsonl
salience: architecture
status: active
subjects:
  - relay-subscriptions
  - subscription-model
  - filter-architecture
supersedes: []
related_claims: []
source_lines:
  - 269-273
captured_at: 2026-06-26T10:03:43Z
---

# Episode: Subscription model: kind-specific expansion → entity-based consolidation

## Prior State

Subscriptions expanded project scope into 4 kind-specific filters per project (profiles/presence/chat/group-state). Filter composition changed whenever session author sets or project membership changed, producing new stable IDs and accumulating orphaned REQs. No unsubscribe mechanism bounded the pool.

## Trigger

User directive proposing a fundamentally different subscription architecture: consolidate to two entity-based REQs (#h for all active channels, #p for all known pubkeys) that remain stable, create new REQs on demand when new channels/agents are added rather than mutating the baseline subscriptions.

## Decision

Replace kind-specific filter expansion with entity-based consolidation: maintain two stable, unchanging generic REQs (#h and #p filters without kind constraints), and create new entity-specific REQs incrementally as new channels or agents are discovered, rather than destroying/recreating baseline filters.

## Consequences

- Eliminates the unbounded kind:0 leak by preventing filter mutations in baseline subscriptions
- Shifts filter composition from daemon-side expansion to relay-side matching (relay applies the filter, not the daemon)
- Changes subscription lifecycle from rebuild-the-entire-union-on-every-session to incremental-add-on-new-entity
- Reduces churn and stable ID collisions by decoupling entity discovery from filter recreation

## Open Tail

- Implementation design: how to track which entity-specific REQs are active and when to create new ones
- Whether #p consolidation should include session pubkeys or only stable identities

## Evidence

- transcript lines 269-273

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-06-26-1-subscription-model-kind-specific-expansion-entity.json`](transcripts/2026-06-26-1-subscription-model-kind-specific-expansion-entity.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-06-26-1-subscription-model-kind-specific-expansion-entity.json`](transcripts/raw/2026-06-26-1-subscription-model-kind-specific-expansion-entity.json)

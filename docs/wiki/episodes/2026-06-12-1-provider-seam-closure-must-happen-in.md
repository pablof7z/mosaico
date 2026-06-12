---
type: episode-card
date: 2026-06-12
session: 0bc06206-1f30-4e35-8373-f31d0f5c1dcc
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/0bc06206-1f30-4e35-8373-f31d0f5c1dcc.jsonl
salience: architecture
status: active
subjects:
  - fabric-provider-seam
  - wire-shape-leaks
  - nostr-codec
supersedes: []
related_claims: []
source_lines:
  - 2087-2111
captured_at: 2026-06-12T20:48:59Z
---

# Episode: Provider seam closure must happen in this task — no deferred wire-shape leaks

## Prior State

Both branches had six places where code above the fabric/provider seam built raw Nostr events inline, violating the architecture rule that wire shapes (kinds, tags) must only live inside a provider. The assistant planned to finish the rebase behavior-preserving, then treat seam-closing as a separate follow-up.

## Trigger

User explicitly rejected deferring seam closure: 'what's the fucking point of having rules if you decide to just break them out of laziness?!'

## Decision

All six wire-shape leaks will be closed within this task, not as a follow-up. Full inventory committed to memory: (1) rpc_user_prompt — inline kind:1 h/p tags, should go through codec as a Mention; (2) rpc_turn_end — TurnReply publishes via Kind1Codec directly, bypassing provider; (3) rpc_propose — inline kind:30023, needs a Proposal domain concept with codec arm; (4) rpc_project_edit — inline kind:9002, belongs in fabric/nip29/lifecycle.rs; (5) doctor — inline kind:1 probe; (6) runtime.rs session engine — publishes Presence/Status/Activity via Kind1Codec directly instead of through the provider.

## Consequences

- A dedicated seam-closing commit must follow the rebase and precede bug fixes
- New domain concepts (Proposal, TurnReply routing through provider, user-prompt-as-Mention) may need to be introduced
- The rebase itself continues behavior-preserving per commit for auditability, but no leak is inherited as technical debt
- The fabric/nip29/lifecycle.rs module already exists and is the correct home for rpc_project_edit

## Open Tail

- Seam-closing commit not yet written; rebase is still in progress (commit ~4 of 12)
- TurnReply and Proposal need domain types and codec arms designed before implementation
- Three tail bugs still pending after rebase and seam closure

## Evidence

- transcript lines 2087-2111


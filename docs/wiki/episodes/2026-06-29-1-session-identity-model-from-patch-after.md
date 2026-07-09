---
type: episode-card
date: 2026-06-29
session: d39d3357-06d0-418a-bdbe-f288a9f9670f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/d39d3357-06d0-418a-bdbe-f288a9f9670f.jsonl
salience: architecture
status: superseded
subjects:
  - session-routing-isolation
  - ordinal-identity-binding
  - session-lifecycle
supersedes: []
related_claims: []
source_lines:
  - 1-44
  - 1836-2102
  - 2253-2391
  - 2342-2407
  - 2408-2702
  - 2845-2862
captured_at: 2026-06-29T10:08:15Z
---

# Episode: Session identity model: from patch-after-birth to born-right ordinal pubkey

## Prior State

Sessions are registered with the base agent pubkey at creation. Ordinal pubkeys (HKDF-derived per-session signing keys) are allocated later but never written to the session row. Route queries use sessions.agent_pubkey, so multiple ordinals appear as one entity and routing p-tags fan out to all of them.

## Trigger

User observes that a mention p-tagged for ordinal-0 is routed to both ordinal-0 and ordinal-1 instances of the same agent. Root-cause diagnosis reveals sessions.agent_pubkey doesn't hold the actual signing identity — both ordinals have the base key in that field.

## Decision

Refactored session registration from patch-after-birth to born-right architecture. Split `register_session` into resolve-or-mint + row-write steps, and reordered `rpc_session_start` to call `select_session_signer` *before* `upsert_session_row`. The session is now created with its correct ordinal pubkey from the start, eliminating the need for after-the-fact `set_session_agent_pubkey` or re-assert preservation hacks.

## Consequences

- Session identity is stable from creation; no out-of-band corrections needed
- Route queries use the row's authoritative pubkey immediately
- Mention isolation works correctly on first write (no re-assert clobbering)
- Reconcile path now only heals stale base-pubkey rows via self-heal, not maintaining live sessions
- Ordinal slot allocation happens after stale-session cleanup, preserving correct ordering invariants

## Open Tail

- Concurrent agent work on agent_label fields overlapped; two commits required (routing fix + reconciliation) to keep the tree buildable and avoid destroying concurrent edits

## Evidence

- transcript lines 1-44
- transcript lines 1836-2102
- transcript lines 2253-2391
- transcript lines 2342-2407
- transcript lines 2408-2702
- transcript lines 2845-2862

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-06-29-1-session-identity-model-from-patch-after.json`](transcripts/2026-06-29-1-session-identity-model-from-patch-after.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-06-29-1-session-identity-model-from-patch-after.json`](transcripts/raw/2026-06-29-1-session-identity-model-from-patch-after.json)

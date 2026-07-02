---
type: episode-card
date: 2026-06-29
session: 3c769f4a-9947-4d7b-a8f5-58355620b951
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/3c769f4a-9947-4d7b-a8f5-58355620b951.jsonl
salience: superseded-reversal
status: superseded
subjects:
  - nip29-removal
  - agent-lifecycle
  - channel-membership
  - session-end
supersedes: []
related_claims: []
source_lines:
  - 1-2
  - 67-76
  - 78-82
  - 84-88
  - 116-118
captured_at: 2026-06-29T09:54:59Z
---

# Episode: Superseded — NIP-29 membership persistence

This historical decision is superseded. Current doctrine is that online presence
is active channel membership; local daemons remove locally managed agent pubkeys
from channel membership when sessions end or become stale.

## Prior State

Agents were removed from channels via NIP-29 nip29_remove_member on session_end. Channel membership was ephemeral (session-bound).

## Trigger

User directive at line 78: 'it shouldn't remove it on session end'. Rationale: NIP-29 membership should represent durable channel membership, not ephemeral session state.

## Decision

Remove nip29_remove_member call from session_end handler. Adopt persistent membership model: agents stay as channel members across session boundaries; kind:30315 TTL is the sole liveness signal.

## Consequences

- Agents persist as channel members when sessions end, not evicted
- Offline/dead agents continue receiving messages sent to their channels
- Resumed agents are already members; no NIP-29 re-add on session restart
- Ordinal reuse path (issue #47) operates without membership churn

## Open Tail

Superseded by the active-membership model.

## Evidence

- transcript lines 1-2
- transcript lines 67-76
- transcript lines 78-82
- transcript lines 84-88
- transcript lines 116-118

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-06-29-1-nip-29-membership-persistence-agents-remain.json`](transcripts/2026-06-29-1-nip-29-membership-persistence-agents-remain.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-06-29-1-nip-29-membership-persistence-agents-remain.json`](transcripts/raw/2026-06-29-1-nip-29-membership-persistence-agents-remain.json)

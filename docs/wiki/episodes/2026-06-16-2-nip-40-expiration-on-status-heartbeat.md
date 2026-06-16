---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: reversal
status: superseded
subjects:
  - nip-40-expiration
  - status-heartbeat
  - kind-30315
supersedes:
  - 2026-06-16-4-nip-40-expiration-on-status-heartbeat
related_claims: []
source_lines:
  - 1315-1327
captured_at: 2026-06-16T11:54:39Z
---

# Episode: NIP-40 expiration on status heartbeat reverses never-expire policy

## Prior State

Kind:30315 status events were published with no expiration — a deliberate design choice (commit 5e7a34d1, 'never expire'). Liveness was tracked by store last_seen refreshed each heartbeat, but the relay event persisted forever, making it impossible for readers to distinguish 'idle for 3 seconds' from 'dead for 3 days.' Combined with identity rotation, this produced unbounded competing never-dying title events.

## Trigger

Architectural research identified that permanent events × identity rotation = unbounded orphaned title events by construction. Codex proposed tombstones (lifecycle: ended/superseded). User explicitly rejected tombstones and directed: 'just include an expiry tag, that's why we publish it as a heartbeat, so we know when the session stopped being active by the lack of an update.'

## Decision

Add NIP-40 ["expiration", <ts>] tag on kind:30315 heartbeat events with TTL = status_ttl (90s). Every 30s heartbeat re-publishes with expiration = now + 90s, so a healthy session re-arms 3× before lapsing. When a session stops, the beats stop, the event expires off the relay, and absence = dead. No tombstones, no supersede events.

## Consequences

- Reverses commit 5e7a34d1's 'never expire' decision
- Finished sessions' titles disappear from the relay ~90s after last heartbeat
- Liveness becomes self-evident from relay state alone (no auxiliary freshness-window bookkeeping needed for peers)
- Second structural backstop for orphan/flip-flop: even if identity churns briefly, the dead address self-expires in 90s
- Saved to memory as tenex-edge-status-expiration-decision.md

## Open Tail

- Tombstone-on-supersede was considered and rejected; may want to revisit for legacy te-* ids already on relay

## Evidence

- transcript lines 1315-1327


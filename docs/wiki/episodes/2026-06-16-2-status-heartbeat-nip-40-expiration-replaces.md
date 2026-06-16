---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: reversal
status: active
subjects:
  - status-liveness
  - nip-40-expiration
  - relay-events
supersedes:
  - 2026-06-16-2-nip-40-expiration-on-heartbeat-replaces
related_claims: []
source_lines:
  - 1314-1326
captured_at: 2026-06-16T12:33:38Z
---

# Episode: Status heartbeat: NIP-40 expiration replaces never-expire policy

## Prior State

Commit 5e7a34d1 deliberately set kind:30315 events to never expire (no NIP-40 expiration tag). Finished/dead sessions remained permanently on the relay with their last title, indistinguishable from merely-idle sessions. Liveness was inferred from the event's continued existence rather than freshness.

## Trigger

User explicitly rejected tombstone and freshness-window approaches, directing: 'just include an expiry tag, that's why we publish it as a heartbeat, so we know when the session stopped being active by the lack of an update.'

## Decision

Add NIP-40 expiration tag to the kind:30315 heartbeat with TTL = status_ttl (90s), re-armed every 30s heartbeat. When a session stops heartbeating, the event expires off the relay within ~90s. Reverses commit 5e7a34d1. No tombstones, no supersede events.

## Consequences

- A finished session's title disappears from the relay ~90s after the last beat — absence = dead
- Stable identity (from the aggregate rearchitecture) keeps titles coherent while alive; expiration cleans up after death
- Even if identity churns and two d-addresses briefly exist, the dead one self-expires in 90s instead of lingering forever
- Local liveness uses the same 90s window (SQLite last_seen), so local and peer liveness finally agree
- Deliberate prior decision (never-expire) is explicitly overturned and documented in project memory

## Open Tail

- Legacy te-* opencode session_ids already on the relay with no expiration — will age out naturally or need a one-time cleanup
- Exact TTL may need tuning (90s gives 3× re-arm margin at 30s heartbeats)

## Evidence

- transcript lines 1314-1326


---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: reversal
status: superseded
subjects:
  - status-event-lifecycle
  - relay-protocol
supersedes:
  - 2026-06-16-2-nip-40-expiration-on-status-heartbeat
related_claims: []
source_lines:
  - 1280-1314
  - 1316-1338
captured_at: 2026-06-16T11:59:49Z
---

# Episode: Status heartbeat events get NIP-40 expiration, reversing never-expire policy

## Prior State

Kind:30315 status events were deliberately set to never expire (commit 5e7a34d1). Combined with unstable session identity, this created unbounded competing permanent title events on the relay — dead sessions' title cards lingered indefinitely, indistinguishable from idle ones.

## Trigger

Architecture research identified that 'never expire' + identity churn = orphaned permanent events by construction. Codex proposed tombstone events (publish lifecycle:ended/superseded terminal states). Opus proposed stable-identity + freshness windows. User explicitly rejected tombstones: 'just include an expiry tag, that's why we publish it as a heartbeat, so we know when the session stopped being active by the lack of an update.'

## Decision

Add NIP-40 ["expiration", <ts>] tag to every kind:30315 heartbeat, with TTL = status_ttl (90s), re-armed on each 30s heartbeat. When the session stops, beats stop, the event expires off the relay, and absence = dead. No tombstone or supersede events needed.

## Consequences

- Reverses commit 5e7a34d1 (never-expire policy)
- Finished sessions' titles disappear from relay ~90s after last heartbeat — the point, since 'no update = not active'
- Combined with stable identity (Card 1), provides belt-and-suspenders: stable identity prevents most orphans, expiration cleans up what remains including legacy te-* IDs already on the relay
- Local liveness still determined by SQLite last_seen (heartbeat within 90s); peer liveness determined by event created_at + expiration
- Decision saved to project memory for persistence across sessions

## Open Tail

*(none)*

## Evidence

- transcript lines 1280-1314
- transcript lines 1316-1338


---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: reversal
status: superseded
subjects:
  - status-event-lifecycle
  - relay-publication
supersedes:
  - 2026-06-16-2-status-heartbeat-events-now-expire-nip
related_claims: []
source_lines:
  - 1279-1316
  - 1324-1327
captured_at: 2026-06-16T12:29:16Z
---

# Episode: NIP-40 expiration on heartbeat replaces never-expire design

## Prior State

Kind:30315 status events were deliberately published with no NIP-40 expiration tag (commit 5e7a34d1 — 'never expire'). Finished/srozen sessions kept their title cards on the relay indefinitely, making liveness indistinguishable from idle (remote who readers seeing stale permanent events).

## Trigger

Architecture planning raised the liveness question: how to distinguish 'idle 3 seconds' from 'dead 3 days' on the relay. Codex proposed tombstone events; Opus proposed stable-identity + freshness windows. User explicitly rejected both in favor of expiration: 'just include an expiry tag, that's why we publish it as a heartbeat, so we know when the session stopped being active by the lack of an update.'

## Decision

Add NIP-40 ["expiration", <ts>] tag to every kind:30315 heartbeat publication. TTL = status_ttl (90s), re-armed each 30s heartbeat (3× safety margin). When a session stops heartbeating, the event self-expires from the relay in ~90s. No tombstones, no supersede events, no freshness-window bookkeeping on permanent events.

## Consequences

- Reverses commit 5e7a34d1's 'never expire' design — finished sessions' titles disappear from the relay ~90s after the last beat
- Liveness becomes self-evident from the relay alone (event present = alive; absent = dead or idle >90s); local and peer liveness finally use the same number (90s = existing who peer-freshness window)
- Second structural backstop for orphan/flip-flop: even if identity churns briefly, the dead d-address self-expires in 90s
- Decision saved to project memory as a deliberate reversal of the prior documented choice

## Open Tail

- Existing permanent events already on the relay will remain until relay-side cleanup or manual deletion — no retroactive expiry
- Need to verify relay supports NIP-40 expiration tags correctly

## Evidence

- transcript lines 1279-1316
- transcript lines 1324-1327


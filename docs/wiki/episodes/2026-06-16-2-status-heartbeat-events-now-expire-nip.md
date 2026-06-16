---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: reversal
status: superseded
subjects:
  - status-expiration
  - nip-40
  - relay-liveness
supersedes:
  - 2026-06-16-2-status-heartbeat-events-get-nip-40
related_claims: []
source_lines:
  - 1314-1326
  - 1296-1312
captured_at: 2026-06-16T12:09:16Z
---

# Episode: Status heartbeat events now expire (NIP-40), reversing never-expire policy

## Prior State

kind:30315 addressable status events were published with no NIP-40 expiration tag (deliberate choice, codified in commit 5e7a34d1). Liveness was undefined: absence of new events didn't mean dead, just quiet. Old sessions lingered on the relay permanently, looking like idle but actually dead.

## Trigger

Architectural research (both Opus and Codex) identified that permanent events × identity churn = unbounded competing title events by construction. The user was presented with two options for handling session end-of-life: tombstones (active terminal event) vs stable-identity + freshness window. The user rejected both framings and directly specified: 'just include an expiry tag, that's why we publish it as a heartbeat, so we know when the session stopped being active by the lack of an update.'

## Decision

Add NIP-40 ["expiration", <ts>] tag to the kind:30315 heartbeat with TTL = status_ttl (currently 90s). Every live session re-publishes with expiration = now + 90s every 30s heartbeat; when the session stops, the beats stop, the event expires off the relay, and absence = dead. No tombstones, no supersede events needed. This explicitly reverses commit 5e7a34d1's 'never expire' design.

## Consequences

- A finished session's title disappears from the relay ~90s after the last beat (the user explicitly accepted this)
- Stale/orphaned sessions self-expire rather than lingering forever as fake-idle
- Second structural backstop for the orphan/flip-flop bug class: even if identity churns and two d-addresses briefly exist, the dead one self-expires in 90s
- Local liveness (last_seen heartbeat within 90s window) and peer liveness (event expiration) now use the same number and same semantics
- Decision saved to project memory as a locked override of the prior never-expire choice

## Open Tail

*(none)*

## Evidence

- transcript lines 1314-1326
- transcript lines 1296-1312


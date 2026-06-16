---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: reversal
status: superseded
subjects:
  - kind-30315-expiration
  - status-liveness
  - nip-40
supersedes:
  - 2026-06-16-3-nip-40-expiration-on-status-heartbeat
related_claims: []
source_lines:
  - 1280-1338
captured_at: 2026-06-16T11:42:46Z
---

# Episode: NIP-40 expiration on status heartbeat — reverses 'never expire'

## Prior State

kind:30315 status events were deliberately published with no expiration (commit 5e7a34d1), so a finished session's title card persisted on the relay indefinitely and readers could not distinguish 'idle for 3 seconds' from 'dead for 3 days'

## Trigger

Discussion of liveness model (tombstones vs aging vs expiration); user decision: 'just include an expiry tag, that's why we publish it as a heartbeat, so we know when the session stopped being active by the lack of an update'

## Decision

Add NIP-40 expiration tag to kind:30315 heartbeat events with TTL = 90s (matching existing status_ttl); re-armed on every 30s heartbeat (3× safety margin); reverses the documented 'never expire' design. No tombstones, no supersede events — liveness is self-evident from the expiration window

## Consequences

- Dead sessions self-expire from relay ~90s after last heartbeat
- Relay event becomes a durable title card while alive, not a liveness signal — liveness is derived from expiration/presence
- Second structural backstop for orphan/flip-flop: even if identity churns briefly, the stale d-address expires in 90s
- Saved as a locked decision in project memory (tenex-edge-status-expiration-decision.md)

## Open Tail

*(none)*

## Evidence

- transcript lines 1280-1338


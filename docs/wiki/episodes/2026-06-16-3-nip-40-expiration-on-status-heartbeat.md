---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: reversal
status: superseded
subjects:
  - status-expiration
  - relay-liveness
supersedes:
  - 2026-06-16-5-relay-liveness-model-expiry-tags-on
related_claims: []
source_lines:
  - 1280-1327
captured_at: 2026-06-16T11:33:11Z
---

# Episode: NIP-40 expiration on status heartbeat reverses 'never expire' policy

## Prior State

Kind:30315 events were deliberately set to never expire (commit 5e7a34d1), meaning finished sessions persisted on the relay indefinitely and liveness was ambiguous (no heartbeat ≠ dead)

## Trigger

Architectural research identified liveness ambiguity as a structural defect; during liveness-model discussion, user explicitly rejected tombstones in favor of expiration: 'just include an expiry tag, that's why we publish it as a heartbeat'

## Decision

Add NIP-40 expiration tag to kind:30315 events with TTL=90s (matching existing status_ttl), re-armed every 30s heartbeat. When the session stops, the beats stop, and the event expires off the relay. No tombstones needed.

## Consequences

- Reverses commit 5e7a34d1 ('never expire')
- Finished sessions disappear from the relay ~90s after last heartbeat
- Liveness becomes self-evident from the relay: present = alive, absent = dead
- Second structural backstop for orphan events (even if identity churns, the dead one self-expires)
- Decision saved to project memory at tenex-edge-status-expiration-decision.md

## Open Tail

*(none)*

## Evidence

- transcript lines 1280-1327


---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: root-cause
status: active
subjects:
  - tenex-edge-daemon
  - nip29-groups
  - membership-cache
supersedes: []
related_claims: []
source_lines:
  - 1553-1557
captured_at: 2026-06-12T08:41:38Z
---

# Episode: Membership warning false positive from stale local cache

## Prior State

The membership warning check in assemble_turn_start_context relied solely on the local SQLite cache (is_group_member) to determine group membership

## Trigger

Running `tenex-edge project add tenex-edge <pubkey>` on the relay-admin machine returned 'blocked: all targets are members already' — the agent was already a member, yet the warning had fired because the remote machine's daemon had an empty local cache

## Decision

Identified but not yet fixed: the check should also query the relay's kind:39002 members event or trust a successful session-start publish, rather than relying solely on local cache

## Consequences

- Cosmetic false positive for fresh daemons until they receive a kind:39002 subscription update
- The agent is functionally able to post — the relay accepts its events
- Future fix needed: cross-reference relay state or gate the warning on publish failure rather than cache absence

## Open Tail

- Need to implement cache-warming from relay subscription or suppress warning when session-start publish succeeded

## Evidence

- transcript lines 1553-1557


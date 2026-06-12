---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: product
status: active
subjects:
  - nip29-group-membership
  - project-add-command
  - agent-onboarding
supersedes: []
related_claims: []
source_lines:
  - 446-558
  - 1027-1177
captured_at: 2026-06-12T11:09:10Z
---

# Episode: NIP-29 group membership management gap — no manual add, no visibility

## Prior State

Agents were added to NIP-29 groups only automatically during session-start via `group_put_user`. There was no standalone CLI command to manually add an agent to a group, and agents that weren't members received no warning — their messages would be silently rejected by the relay.

## Trigger

User asked whether new agents request access to the NIP-29 group and whether tenex-edge provides a way to add them. Investigation confirmed no `add-member` CLI subcommand existed, and the session-start auto-add path silently fails if the operator key isn't admin.

## Decision

Added `tenex-edge project add <project> <pubkey-or-npub-or-nip05>` CLI command and daemon RPC (`project_add`) that resolves npub/nip05 to hex pubkey (via HTTP NIP-05 lookup) and publishes a kind:9000 `put-user` event. Also added a first-turn membership warning in `assemble_turn_start_context` that checks `is_group_member` and surfaces an actionable message with the exact command to run.

## Consequences

- Operators can now manually add agents to NIP-29 groups from any machine with relay admin access
- New agents that fail auto-enrollment now get an explicit warning on their first turn
- The warning includes the exact `tenex-edge project add` command with the agent's pubkey
- NIP-05 resolution requires `reqwest` (added as dependency)

## Open Tail

- The membership check relies on local cache; if cache is empty (fresh daemon), the warning fires even if the agent is already a relay-side member (observed in practice)

## Evidence

- transcript lines 446-558
- transcript lines 1027-1177


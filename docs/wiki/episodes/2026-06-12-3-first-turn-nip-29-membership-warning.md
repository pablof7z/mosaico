---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: product
status: active
subjects:
  - tenex-edge-hooks
  - nip29-groups
  - agent-awareness
supersedes: []
related_claims: []
source_lines:
  - 1094-1097
captured_at: 2026-06-12T08:41:38Z
---

# Episode: First-turn NIP-29 membership warning for unauthorized agents

## Prior State

If ensure_group_and_membership failed silently at session-start (e.g. operator key not admin on that relay), the agent had zero visibility into its non-member status — messages would be rejected by the relay with no explanation

## Trigger

Identified during `project add` implementation as a necessary complement: agents need to know when they're not group members and what to do about it

## Decision

Added a check in `assemble_turn_start_context` (turn.rs) that checks `is_group_member` for the session's agent on the first UserPromptSubmit. If not a member, emits a WARNING block with the exact `tenex-edge project add` command needed, including the agent's hex pubkey

## Consequences

- Agents now receive actionable feedback when they lack group membership
- Validated end-to-end on remote machine — the warning appeared in Claude Code's hook output
- The warning includes the exact CLI command to resolve the issue

## Open Tail

- Warning fires as false positive when local cache is empty but relay already has the member — see separate arc

## Evidence

- transcript lines 1094-1097


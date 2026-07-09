---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: product
status: active
subjects:
  - mgmt-channel-privacy
  - backend-traffic-filtering
supersedes:
  - 2026-07-09-a62822c5d09c-fbf565a0-2-backend-mgmt-traffic-filtered-from-agent
related_claims: []
source_lines:
  - 78-87
  - 1109-1110
captured_at: 2026-07-09T14:50:00Z
---

# Episode: Mgmt/backend exchanges filtered from agent-visible channel reads

## Prior State

Management commands (e.g. `list agents`) and their replies were published as ordinary kind:9 chat events to the relay and mirrored into local SQLite. No access-control or visibility filtering existed — any agent reading the channel via `chat_read` would see mgmt round-trips between the user and the daemon backend.

## Trigger

User observed that `tenex-edge channel read` showed a communication between the user and the mgmt agent, which was supposed to be invisible to agents.

## Decision

`handle_chat_read` now filters out backend-authored and backend-p-tagged rows (both in the initial batch and the `--live` tail), reusing the existing `is_backend_traffic` check from `fabric_context::messages` (bumped to `pub(crate)`). Mgmt round-trips no longer appear in agent channel reads.

## Consequences

- Agents reading a channel no longer see management command/reply exchanges, preserving the intended privacy of mgmt operations.
- The same backend-traffic predicate now guards both the hook/awareness snapshot path and the direct `chat_read` RPC path — consistent visibility invariant.

## Open Tail

*(none)*

## Evidence

- transcript lines 78-87
- transcript lines 1109-1110

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-2-mgmt-backend-exchanges-filtered-from-agent.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-2-mgmt-backend-exchanges-filtered-from-agent.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-2-mgmt-backend-exchanges-filtered-from-agent.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-2-mgmt-backend-exchanges-filtered-from-agent.json)

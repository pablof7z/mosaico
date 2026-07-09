---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: architecture
status: superseded
subjects:
  - channel-read
  - backend-traffic-filter
  - mgmt-visibility
  - is-backend-traffic
supersedes: []
related_claims: []
source_lines:
  - 78-87
  - 175-181
  - 627-695
  - 1109-1115
captured_at: 2026-07-09T14:42:06Z
---

# Episode: Backend/mgmt traffic filtered from agent-visible channel reads

## Prior State

Management commands (e.g. `list agents`) were published as real kind:9 NIP-29 chat events to the same channel group and mirrored into SQLite. The `is_backend_traffic` filter existed in `fabric_context/messages.rs` and correctly guarded the hook/awareness snapshot, but was never wired into `chat_read_tail.rs`. Any agent reading the channel would see mgmt round-trips.

## Trigger

User observed that `tenex-edge channel read` showed a communication between the user and the mgmt agent, which was supposed to be invisible to agents.

## Decision

Apply the existing `is_backend_traffic` filter in `chat_read_tail.rs` for both initial batch reads and `--live` tail streams, filtering out rows where the author pubkey or any p-tagged recipient pubkey is flagged as backend. Bumped `is_backend_traffic` to `pub(crate)` visibility to reuse it cross-module.

## Consequences

- Mgmt round-trips (list agents, list sessions, etc.) are no longer visible when agents read channels via `channel read` or tail.
- The visibility invariant — 'backend traffic is invisible to agent channel reads' — now holds uniformly across both the hook/awareness path and the CLI channel-read path.
- Regression tests added for both author-is-backend and recipient-is-backend cases.

## Open Tail

*(none)*

## Evidence

- transcript lines 78-87
- transcript lines 175-181
- transcript lines 627-695
- transcript lines 1109-1115

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-2-backend-mgmt-traffic-filtered-from-agent.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-2-backend-mgmt-traffic-filtered-from-agent.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-2-backend-mgmt-traffic-filtered-from-agent.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-2-backend-mgmt-traffic-filtered-from-agent.json)

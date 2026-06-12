---
type: episode-card
date: 2026-06-12
session: da7ab617-89fb-4b68-9e2d-3f251fe6c1d9
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/da7ab617-89fb-4b68-9e2d-3f251fe6c1d9.jsonl
salience: product
status: active
subjects:
  - tenex-edge-who
  - remote-host-display
supersedes: []
related_claims: []
source_lines:
  - 1-1
  - 65-65
  - 71-76
  - 102-147
captured_at: 2026-06-12T08:48:05Z
---

# Episode: who command shows hostname instead of generic (remote) label

## Prior State

The `who` command rendered remote agents with a static label `(remote)`, giving no information about which machine the remote was running on. The `WhoRow` struct already carried a `host` field but `render_who_row` / `render_who_plain` ignored it in favor of the generic string.

## Trigger

User reported that `tenex-edge who` only shows '(remote)' without identifying the computer, making it impossible to distinguish between multiple remotes or know where a remote agent is actually running.

## Decision

Changed the render functions to display the actual hostname from `WhoRow.host` (e.g., `(tower)`) instead of the generic `(remote)` string.

## Consequences

- Users can now identify which specific machine a remote agent is running on
- Multiple remotes on different hosts become distinguishable at a glance
- Tests that asserted the literal string '(remote)' were updated to assert the actual hostname (e.g., '(tower)')

## Open Tail

*(none)*

## Evidence

- transcript lines 1-1
- transcript lines 65-65
- transcript lines 71-76
- transcript lines 102-147


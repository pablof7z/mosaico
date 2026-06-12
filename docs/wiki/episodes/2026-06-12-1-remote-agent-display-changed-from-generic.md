---
type: episode-card
date: 2026-06-12
session: da7ab617-89fb-4b68-9e2d-3f251fe6c1d9
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/da7ab617-89fb-4b68-9e2d-3f251fe6c1d9.jsonl
salience: product
status: active
subjects:
  - who-command
  - remote-display
  - hostname-visibility
supersedes: []
related_claims: []
source_lines:
  - 1-1
  - 65-65
  - 218-233
  - 192-192
captured_at: 2026-06-12T11:08:30Z
---

# Episode: Remote agent display changed from generic label to hostname

## Prior State

The `who` command rendered remote agents with a generic `(remote)` annotation, giving users no indication of which machine the remote session was on

## Trigger

User reported: 'tenex-edge who only shows (remote) but it doesn't tell me on what computer the remote is running'

## Decision

Replace the static `(remote)` string with the actual hostname from `WhoRow.host`, so remote agents now display e.g. `(tower)` instead of the undifferentiated `(remote)`

## Consequences

- Users can now identify which specific machine a remote agent is running on
- All test assertions checking for the literal string `(remote)` had to be updated to match the new hostname-based format
- The §8e comment in render.rs was updated to reflect the new behavior: 'gets ` (hostname)` instead of ` (remote)`'

## Open Tail

*(none)*

## Evidence

- transcript lines 1-1
- transcript lines 65-65
- transcript lines 218-233
- transcript lines 192-192


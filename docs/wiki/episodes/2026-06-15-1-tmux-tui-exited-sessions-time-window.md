---
type: episode-card
date: 2026-06-15
session: 215d979a-a054-4e2b-b349-851e0d874d6d
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/215d979a-a054-4e2b-b349-851e0d874d6d.jsonl
salience: product
status: active
subjects:
  - tmux-tui-exited-sessions-filter
supersedes:
  - 2026-06-14-1-tmux-tui-exited-sessions-time-window
related_claims: []
source_lines:
  - 1-136
captured_at: 2026-06-15T08:02:57Z
---

# Episode: Tmux TUI exited-sessions time-window filter replaces boolean toggle

## Prior State

The exited sessions panel used a simple boolean show/hide toggle with no time-based filtering. All resumable dead sessions were shown regardless of age.

## Trigger

User requested ability to filter exited sessions by hours (default 4h) and easily adjust the window with + and - keys.

## Decision

Replaced show_exited: bool with exited_hours: Option<u64> (None = hidden, Some(h) = filter to h hours). Added +/=/- key handlers with stepped increments (+1h to 12h, +6h to 48h, +24h beyond; reverse for decrease, minimum 1h). Section header shows active window (e.g. 'Exited sessions (past 4h)'), help line reflects current state.

## Consequences

- Exited sessions panel now scopes to a configurable time window, reducing noise from stale sessions
- Stepped increments prevent tedious single-hour scrolling at large ranges
- filter_resumable signature changed from (data, project, show_exited: bool) to (data, project, exited_hours: Option<u64>)

## Open Tail

*(none)*

## Evidence

- transcript lines 1-136


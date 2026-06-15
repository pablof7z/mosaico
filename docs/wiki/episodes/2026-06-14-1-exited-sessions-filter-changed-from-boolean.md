---
type: episode-card
date: 2026-06-14
session: 215d979a-a054-4e2b-b349-851e0d874d6d
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/215d979a-a054-4e2b-b349-851e0d874d6d.jsonl
salience: product
status: superseded
subjects:
  - tmux-tui-exited-sessions-filter
supersedes:
  - 2026-06-14-2-exited-sessions-hidden-by-default-in
related_claims: []
source_lines:
  - 1-135
captured_at: 2026-06-14T19:03:11Z
---

# Episode: Exited-sessions filter changed from boolean toggle to adjustable time window

## Prior State

The tmux TUI offered only a binary `show_exited: bool` toggle — exited sessions were either hidden or shown with no time filtering.

## Trigger

User explicitly requested the ability to see past sessions within X hours, adjust that window with +/- keys, and default to 4 hours.

## Decision

Replaced the boolean toggle with a time-windowed filter: `exited_hours: u64` (default 4), controlled by `+`/`-` keys with stepped increments (+1h up to 12h, +6h up to 48h, +24h beyond, minimum 1h). The `show_exited` toggle still controls visibility, but the exited list is now filtered by the hours window.

## Consequences

- Section header now displays active window (e.g. 'Exited sessions (past 4h)')
- Help line reflects current hours when exited panel is visible (e.g. '[-/+] 4h')
- draw_tui and filter_resumable signatures now carry exited_hours parameter
- First enable of exited panel starts at the 4h default

## Open Tail

*(none)*

## Evidence

- transcript lines 1-135


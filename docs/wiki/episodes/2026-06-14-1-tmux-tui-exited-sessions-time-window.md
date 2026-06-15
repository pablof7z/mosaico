---
type: episode-card
date: 2026-06-14
session: 215d979a-a054-4e2b-b349-851e0d874d6d
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/215d979a-a054-4e2b-b349-851e0d874d6d.jsonl
salience: product
status: superseded
subjects:
  - tmux-tui-exited-filter
  - session-time-window
supersedes:
  - 2026-06-14-1-exited-sessions-filter-changed-from-boolean
related_claims: []
source_lines:
  - 1-135
captured_at: 2026-06-14T19:18:31Z
---

# Episode: Tmux TUI exited sessions time-window filter

## Prior State

Exited sessions in the tmux TUI were shown without time-based filtering; no user control over the recency window

## Trigger

User requested ability to see past sessions from the past X hours with an easily adjustable filter, defaulting to 4 hours

## Decision

Added a time-window filter for exited sessions with a default of 4 hours, adjustable via `+`/`-`/`=` keys with step-wise increments (1h up to 12h, 6h up to 48h, 24h beyond), minimum 1h

## Consequences

- Section header shows active window e.g. 'Exited sessions (past 4h)'
- Help line adapts: '[e] hide exited  [-/+] 4h' when visible, '[e] show exited' when hidden
- `[e]` toggle defaults to 4h window on first enable

## Open Tail

*(none)*

## Evidence

- transcript lines 1-135


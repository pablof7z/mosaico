---
type: episode-card
date: 2026-06-15
session: 9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf.jsonl
salience: product
status: superseded
subjects:
  - tenex-edge-tmux-session-switching
  - tmux-popup-binding
supersedes:
  - 2026-06-15-2-session-switching-ux-adopts-phased-approach
related_claims: []
source_lines:
  - 85-113
  - 126-126
  - 137-169
captured_at: 2026-06-15T07:15:29Z
---

# Episode: Session-switching UX: popup approach built as interim toward persistent sidebar

## Prior State

No mechanism existed to switch between project sessions while attached inside a tmux session; user had to detach and re-enter the TUI.

## Trigger

User asked what sidebar UX was technically possible; assistant proposed three options (persistent split-pane sidebar, popup on keypress, tmux's own session chooser).

## Decision

Build Option B (popup on Alt-t via tmux display-popup -w 90% -h 80% -E 'tenex-edge tmux') as an interim evaluation, reusing the existing TUI unchanged. inject_popup_binding() is called on every attach. User expressed intent to ultimately build Option A (persistent split-pane sidebar).

## Consequences

- Alt-t (Option-t on macOS) keybinding injected into tmux root key table on every attach via inject_popup_binding()
- Popup reuses the full existing TUI — no new subcommand or render code needed
- Binding injection is idempotent (silently swallows errors, let _ = on .status())
- Popup opens as floating overlay covering 90% width / 80% height; Enter switches session, Esc/q dismisses

## Open Tail

- Option A (persistent split-pane sidebar with tenex-edge tmux sidebar subcommand, pane lifecycle management, current-session highlighting) is the intended final direction
- Ratatui migration may simplify sidebar implementation within a single process rather than a separate split-pane process

## Evidence

- transcript lines 85-113
- transcript lines 126-126
- transcript lines 137-169


---
type: episode-card
date: 2026-06-15
session: 9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf.jsonl
salience: product
status: superseded
subjects:
  - tenex-edge-session-switching
  - tmux-sidebar
supersedes: []
related_claims: []
source_lines:
  - 1-3
  - 85-113
  - 126-135
captured_at: 2026-06-15T07:09:35Z
---

# Episode: Session-switching UX adopts phased approach: popup prototype then persistent sidebar

## Prior State

No dedicated session-switching UX within attached tmux sessions — users rely on tmux's built-in Ctrl-b s or manual detach/re-attach

## Trigger

User asks about showing a sidebar with other project sessions for quick switching; assistant proposes three options (A: persistent split-pane, B: display-popup, C: tmux native chooser); user directs 'build the popup approach so I can see, but pretty sure we'll end up building option A'

## Decision

Build Option B (tmux display-popup reusing existing TUI) first as an evaluable prototype, with planned migration to Option A (persistent split-pane sidebar via tmux split-window + dedicated sidebar subcommand with pane lifecycle management)

## Consequences

- Option B requires zero new render code — reuses existing TUI inside tmux display-popup
- Option A will require a new sidebar subcommand, pane lifecycle management (no double-creation, cleanup on session end), current-session highlighting
- Ratatui migration makes Option A's two-pane single-process layout easier to build

## Open Tail

- Option B prototype in progress via background agent in git worktree
- Option A implementation not yet started — depends on ratatui migration landing and Option B evaluation

## Evidence

- transcript lines 1-3
- transcript lines 85-113
- transcript lines 126-135


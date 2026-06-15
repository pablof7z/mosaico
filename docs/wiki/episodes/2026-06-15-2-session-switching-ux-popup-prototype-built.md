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
  - 2026-06-15-2-session-switching-ux-popup-approach-built
related_claims: []
source_lines:
  - 85-113
  - 126-135
  - 137-176
  - 547-582
captured_at: 2026-06-15T07:31:31Z
---

# Episode: Session switching UX: popup prototype built, sidebar planned

## Prior State

No mechanism existed to switch between project sessions while attached inside a tmux session; user had to detach and re-enter the TUI manually

## Trigger

User asked what UX is technically possible for a session sidebar; three options identified (A: persistent split-pane sidebar, B: popup on keypress, C: tmux's own session chooser); user directed building Option B as prototype but expressed intent to ultimately build Option A

## Decision

Implement Option B (tmux display-popup) as a testable prototype: inject_popup_binding() registers Alt-t in root key table to open tenex-edge tmux as a floating popup (90%×80%), injected at both attach_pane_blocking() and attach_pane() entry points

## Consequences

- Popup code lives in separate worktree branch worktree-agent-a7aeac15bf94749ea, not yet merged to master
- Binding registered as M-t in tmux root key table — but Option-t on macOS sends dagger character (†) instead of M-t, making the keybinding non-functional on default macOS terminal configs
- Popup approach reuses existing TUI unchanged (no new subcommand or render code needed)
- If Option A (persistent sidebar) is built, it will require a dedicated sidebar subcommand, pane lifecycle management, and ratatui two-pane layout

## Open Tail

- Alt-t binding broken on macOS due to Option key behavior — needs either terminal reconfiguration (Esc+ mode) or binding change to prefix-key sequence
- User explicitly expects to build Option A (persistent sidebar) as the final UX direction
- Popup branch not yet merged; needs keybinding fix and user acceptance before merge or abandonment in favor of sidebar

## Evidence

- transcript lines 85-113
- transcript lines 126-135
- transcript lines 137-176
- transcript lines 547-582


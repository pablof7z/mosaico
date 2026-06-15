---
type: episode-card
date: 2026-06-15
session: 9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf.jsonl
salience: product
status: superseded
subjects:
  - tenex-edge-tmux-session-switching
supersedes:
  - 2026-06-15-2-session-switching-ux-popup-prototype-built
related_claims: []
source_lines:
  - 85-113
  - 126-127
  - 551-552
  - 616-635
  - 637-731
captured_at: 2026-06-15T07:41:37Z
---

# Episode: Always-visible session-switcher sidebar adopted over popup approach

## Prior State

No way to switch between project sessions while attached to a tmux session — user had to detach and re-enter the TUI list. Three options were proposed: A (persistent split-pane sidebar), B (popup on keypress), C (tmux built-in session chooser).

## Trigger

User tested Option B (popup via display-popup bound to Alt-t → Ctrl-b s after macOS key-mapping issue), then explicitly directed: 'now build the sidebar on the master so that its always visible.'

## Decision

Option A: always-visible sidebar as a 32-column left pane injected into each agent's own tmux window on attach, running 'tenex-edge tmux sidebar' as its own event loop. Current session highlighted (cyan bold + ►). Alt-s/Alt-a to move focus between sidebar and agent pane. Enter on another session calls ensure_sidebar there then switch-client so sidebar persists across switches.

## Consequences

- New TmuxAction::Sidebar subcommand with --session and --project optional args
- LiveRow struct gained project field for sidebar filtering by project
- ensure_sidebar() idempotently injects sidebar pane (checks pane_start_command for 'tmux sidebar')
- bind_sidebar_keys() registers M-s select-pane -L and M-a select-pane -R
- Sidebar uses 2s polling refresh via fetch_tui_data(), truncates slug to available column width
- Popup approach (Option B, worktree-agent-a7aeac15bf94749ea) is historical/prototype — not merged to master
- Sidebar worktree needs rebase onto ratatui master before merge

## Open Tail

- Sidebar worktree not yet merged to master — rebased cleanly but untested against ratatui codebase (sidebar may still use crossterm imports removed by the ratatui migration)

## Evidence

- transcript lines 85-113
- transcript lines 126-127
- transcript lines 551-552
- transcript lines 616-635
- transcript lines 637-731


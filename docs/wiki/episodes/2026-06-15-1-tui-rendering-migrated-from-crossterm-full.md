---
type: episode-card
date: 2026-06-15
session: 9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf.jsonl
salience: architecture
status: active
subjects:
  - tenex-edge-tmux-tui
  - ratatui-migration
supersedes:
  - 2026-06-15-1-tui-rendering-migrated-from-manual-crossterm
related_claims: []
source_lines:
  - 1-82
  - 115-124
  - 193-214
  - 297-331
  - 538-545
captured_at: 2026-06-15T07:31:31Z
---

# Episode: TUI rendering migrated from crossterm full-clear to ratatui

## Prior State

TUI used crossterm with manual full-screen clear-and-repaint every frame (MoveTo(0,0) + Clear(ClearType::All)), no widget tree, no cell diffing, no double-buffer — caused potential flicker on rapid repaints

## Trigger

User asked whether the TUI was a proper ratatui app or just re-rendering; investigation confirmed the naive full-clear pattern, prompting the directive to migrate to ratatui

## Decision

Replace draw_tui()/draw_search() with ratatui render_main()/render_search() backed by Terminal<CrosstermBackend>; adopt ratatui 0.30.1 with crossterm_0_28 feature; use Style/Span/Modifier instead of owo_colors for styled output

## Consequences

- Double-buffered rendering eliminates full-clear flicker
- ratatui dependency added to Cargo.toml with crossterm_0_28 feature to match existing crossterm version
- owo_colors styling calls replaced with ratatui Style/Color/Modifier helpers throughout tmux_cli.rs
- ratatui_term.clear() called after TuiTerminal::resume() to invalidate double-buffer on return from tmux attach
- Simplifies future sidebar layout work since ratatui provides Layout/Constraint primitives

## Open Tail

- Ratatui branch agent also silently changed SEVEN_DAYS→TWELVE_HOURS threshold and tab ordering to session-count-descending — these behavior changes rode in with the migration and should be reviewed for intent

## Evidence

- transcript lines 1-82
- transcript lines 115-124
- transcript lines 193-214
- transcript lines 297-331
- transcript lines 538-545


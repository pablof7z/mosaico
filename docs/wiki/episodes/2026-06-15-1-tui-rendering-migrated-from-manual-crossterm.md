---
type: episode-card
date: 2026-06-15
session: 9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf.jsonl
salience: architecture
status: superseded
subjects:
  - tenex-edge-tui-rendering
  - ratatui-adoption
supersedes:
  - 2026-06-15-1-tenex-edge-tui-migrates-from-manual
related_claims: []
source_lines:
  - 74-81
  - 115-115
  - 193-211
  - 299-326
captured_at: 2026-06-15T07:15:29Z
---

# Episode: TUI rendering migrated from manual crossterm redraw to ratatui

## Prior State

TUI used raw crossterm with manual full-clear redraw every frame: build Vec<String>, execute!(MoveTo(0,0), Clear(ClearType::All)), write each line, flush. No widget tree, no cell diffing, no double-buffer — caused flash on rapid repaints.

## Trigger

User asked whether the TUI was a proper ratatui app or just re-rendering everything; on learning it was crude full-clear, directed migration to ratatui.

## Decision

Adopt ratatui 0.30.1 (crossterm_0_28 feature, matching existing crossterm dep) as the TUI framework. Replace draw_tui() and draw_search() with ratatui render functions (render_main, render_scrolled_body, render_search) using typed Span/Style/Paragraph widgets, double-buffered Terminal::draw, and scroll offset via Paragraph::scroll.

## Consequences

- Screen flash on repaints eliminated by ratatui's dirty-cell diffing and double-buffer
- ratatui dependency added to Cargo.toml with crossterm_0_28 feature flag (no version conflict)
- Enables future complex layouts (e.g. persistent sidebar) that would be impractical with manual redraw
- TuiTerminal::resume() now calls ratatui_term.clear() to invalidate double-buffer after returning from tmux attach

## Open Tail

*(none)*

## Evidence

- transcript lines 74-81
- transcript lines 115-115
- transcript lines 193-211
- transcript lines 299-326


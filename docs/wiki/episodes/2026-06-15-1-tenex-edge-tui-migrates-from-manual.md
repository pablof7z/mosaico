---
type: episode-card
date: 2026-06-15
session: 9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf.jsonl
salience: architecture
status: superseded
subjects:
  - tenex-edge-tui-rendering
  - ratatui-migration
supersedes: []
related_claims: []
source_lines:
  - 1-81
  - 115-124
captured_at: 2026-06-15T07:09:35Z
---

# Episode: tenex-edge TUI migrates from manual crossterm redraw to ratatui

## Prior State

TUI built on raw crossterm: builds Vec<String> each frame, clears entire screen with Clear(ClearType::All), repaints from scratch — no widget tree, no dirty-cell tracking, no double-buffer, causing flash on rapid repaints

## Trigger

User question revealed the full-clear redraw pattern; explicit directive to 'launch a sonnet agent to update it to ratatui in a git worktree and merge once ready'

## Decision

Replace crossterm manual full-clear redraw with ratatui widget-based rendering with proper double-buffered rendering and dirty-cell diffing

## Consequences

- Eliminates screen flash on rapid repaints
- Enables more complex TUI layouts (prerequisite for persistent sidebar)
- Existing logic (tab nav, fuzzy search, attach/resume/spawn, polling refresh) preserved unchanged

## Open Tail

- Migration in progress via background agent in git worktree, merge pending

## Evidence

- transcript lines 1-81
- transcript lines 115-124


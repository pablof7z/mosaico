---
type: episode-card
date: 2026-06-14
session: 656e1e6b-2569-42da-8844-768a5e74788e
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/656e1e6b-2569-42da-8844-768a5e74788e.jsonl
salience: product
status: superseded
subjects:
  - tenex-edge
  - tmux-tui
  - project-tabs
supersedes:
  - 2026-06-14-1-tmux-tui-redesign-project-tabs-hidden
related_claims: []
source_lines:
  - 1-8
  - 379-383
captured_at: 2026-06-14T18:52:26Z
---

# Episode: Tmux TUI project-based session navigation with tabs, filtering, and search

## Prior State

Tmux TUI showed sessions flat without project separation; users could not distinguish which project a session belonged to, and all projects were listed equally regardless of activity.

## Trigger

User requested project separation via tabs (line 1), then refined with three requirements: prioritize live projects, hide inactive (>7d) projects, add fuzzy search (lines 379-383).

## Decision

Sessions are now organized by project tabs (←/→ to switch). Tab ordering prioritizes projects with live sessions first (alphabetically), then recently active (within 7 days). Projects with no activity in the past 7 days are hidden from the tab bar entirely. Pressing '/' opens a fuzzy search overlay to find and jump to any project (including hidden ones). In the 'All' tab, sessions display as slug@project for disambiguation. Label renames: 'Spawnable (no session)' → 'Agents', '[spawnable via claude]' → '[claude]'.

## Consequences

- Stale/inactive projects no longer clutter the tab bar by default
- Hidden projects can still be reached via '/' search and are temporarily injected into visible tabs when selected
- Hidden projects re-hide on next periodic refresh unless activity resumes
- Agents section appears across all project tabs since agents are cross-project

## Open Tail

*(none)*

## Evidence

- transcript lines 1-8
- transcript lines 379-383


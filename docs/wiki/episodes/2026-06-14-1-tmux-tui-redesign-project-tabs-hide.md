---
type: episode-card
date: 2026-06-14
session: 656e1e6b-2569-42da-8844-768a5e74788e
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/656e1e6b-2569-42da-8844-768a5e74788e.jsonl
salience: product
status: active
subjects:
  - tmux-tui
  - session-display
  - project-navigation
supersedes:
  - 2026-06-14-1-tmux-tui-project-based-session-navigation
related_claims: []
source_lines:
  - 1-8
  - 315-320
  - 379-384
  - 513-519
captured_at: 2026-06-14T19:09:07Z
---

# Episode: tmux TUI redesign: project tabs, hide exited, concise labels

## Prior State

TUI showed all sessions in a flat list without project separation; exited sessions displayed by default; labels used verbose forms like 'Spawnable (no session)' and '[spawnable via claude]'

## Trigger

User correction: sessions across all projects were mixed together with no way to identify which project a session belongs to; exited sessions cluttering the view; verbose labels were unnecessarily wordy

## Decision

Redesigned TUI with: (1) project tabs (←/→ to switch) grouping sessions by project with 'All' as default; (2) exited sessions hidden by default, toggled with 'e'; (3) 'Agents' replacing 'Spawnable (no session)', '[claude]' replacing '[spawnable via claude]'; (4) tabs ordered by live-activity (projects with online sessions first), then recently-active (within 7 days); projects with no activity in 7 days hidden entirely from tab bar; (5) '/' fuzzy-search overlay to find and jump to projects including hidden ones

## Consequences

- In 'All' tab, session labels show slug@project so users can identify origin
- Agents section is cross-project and always visible regardless of active tab
- Projects inactive for >7 days only surface via fuzzy search; selecting one temporarily injects it into visible tabs until next refresh
- Tab ordering recomputed on every 2s refresh cycle

## Open Tail

- The 7-day inactivity threshold is hardcoded; may need configuration later
- Temporarily-injected hidden projects re-hide on next periodic refresh

## Evidence

- transcript lines 1-8
- transcript lines 315-320
- transcript lines 379-384
- transcript lines 513-519


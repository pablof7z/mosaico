---
title: Tenex-Edge TUI
slug: tenex-edge-tui
topic: tenex-edge
summary: The TUI is built with ratatui (version 0.30.1, default-features disabled, features crossterm_0_28 and macros enabled) using the crossterm 0.28 backend for doubl
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-14
updated: 2026-06-15
verified: 2026-06-14
compiled-from: conversation
sources:
  - session:9f7f245f-0fad-4211-a86b-95ea3cbb532e
  - session:656e1e6b-2569-42da-8844-768a5e74788e
  - session:622711fa-5176-4580-b311-d66446c2924b
  - session:9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
  - session:9c78b46a-3169-42eb-84c1-228a6c2f6589
---

# Tenex-Edge TUI

## Scrolling Behavior

The TUI is built with ratatui (version 0.30.1, default-features disabled, features crossterm_0_28 and macros enabled) using the crossterm 0.28 backend for double-buffered rendering with dirty-cell tracking instead of manual full-clear redraws. The ratatui migration replaces draw_tui() with render_main() and render_scrolled_body(), and replaces draw_search() with render_search(), while keeping all non-drawing logic (tab computation, fuzzy search, key handling, attach/spawn/resume flows, daemon RPC calls, TuiTerminal guard) unchanged. After returning from a tmux attach, ratatui_term.clear() is called to invalidate the double-buffer and force a full redraw. It groups sessions by project in tabs, navigable via Left/Right arrow keys, with an [All] default tab showing all sessions. In the [All] tab, session labels display in slug@project format so the project of each session is identifiable. The list scrolls — render_scrolled_body renders only lines that fit the terminal height, keeps the selected row in view, and shows ↑N more above / ↓N more below indicators. Exited sessions are hidden by default and toggled visible by pressing 'e'; when they are visible, the help line updates to show [e] hide exited. The 'Spawnable (no session)' label is renamed to 'Agents'. The '[spawnable via claude]' label is renamed to '[claude]'. Agents appear in all project tabs since they are cross-project. The "[no tmux]" indicator is not displayed; all live sessions render with the same colorized styling. Pressing Enter on a spawnable agent spawns it, replacing the previous [n] key binding, and the hint bar displays "[↵] attach/spawn". When spawning a new tmux session from a project tab, the session is created within the selected project's directory, not the TUI process's current working directory; when spawning from the [All] tab (where no project filter is active), the session directory falls back to the current working directory.

<!-- citations: [^9f7f2-15] [^9f7f2-21] [^9f7f2-25] [^656e1-2] [^656e1-5] [^62271-2] [^9bab9-1] [^9bab9-4] [^9c78b-1] -->
## Project Tab Priority and Visibility

Project tabs are sorted by live session count descending (most active first) with alphabetical tiebreaker. Projects with no live sessions and no activity in the past 12 hours are hidden by default. (Previously: the inactivity threshold was seven days.) Selecting a hidden project via fuzzy search temporarily injects it into the visible tabs; it re-hides on the next periodic refresh unless activity resumes.

<!-- citations: [^656e1-3] [^656e1-6] [^62271-3] [^9bab9-7] -->
## Fuzzy Search

Pressing '/' opens a fuzzy search overlay to filter projects by case-insensitive substring. In the overlay, Up/Down arrows navigate results, Enter jumps to the selected project tab, and Escape cancels.

<!-- citations: [^656e1-4] [^656e1-7] -->

## Session Switching

When attached to a tmux session, pressing Alt-t opens a floating tmux display-popup at 80% width and 80% height running the TUI in --popup mode for quick session switching. The popup session-switcher is built in a git worktree for evaluation, with the expectation that a persistent split-pane sidebar will likely be the final approach. If the persistent split-pane sidebar is built, it will be implemented as a sidebar subcommand launched via `tmux split-window`, displaying the project's sessions with the current session highlighted, and switching sessions via `tmux switch-client`.

<!-- citations: [^9bab9-2] [^9bab9-6] [^9bab9-10] -->

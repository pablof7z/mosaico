---
type: episode-card
date: 2026-06-15
session: 9c78b46a-3169-42eb-84c1-228a6c2f6589
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9c78b46a-3169-42eb-84c1-228a6c2f6589.jsonl
salience: root-cause
status: superseded
subjects:
  - tmux-spawn-project-resolution
  - tui-project-tab-context
supersedes: []
related_claims: []
source_lines:
  - 7-7
  - 154-162
  - 196-229
captured_at: 2026-06-15T08:12:54Z
---

# Episode: tmux TUI spawn resolves project from selected tab, not cwd

## Prior State

When spawning a new tmux session from the TUI on a spawnable agent row, the project was always resolved from the TUI process's current working directory via `std::env::current_dir()`, ignoring which project tab was selected in the TUI. The daemon's `project_abs_path` would correctly map the project name to its directory, but it was receiving the wrong project name.

## Trigger

User reported that creating a new session spawns it within the pwd instead of within the selected project's directory. Root cause identified in `src/cli/tmux_cli.rs:1155` — the Enter/spawn handler used `crate::project::resolve(&std::env::current_dir().unwrap_or_default())` unconditionally.

## Decision

Changed the spawn handler to use the active project tab filter (`pf`) as the project name when a specific tab is selected, falling back to cwd-resolution only on the unfiltered 'all projects' tab where no tab-level project context exists: `pf.map(str::to_string).unwrap_or_else(|| crate::project::resolve(&std::env::current_dir().unwrap_or_default()))`.

## Consequences

- Spawning from a specific project tab now correctly creates the session in that project's directory
- On the 'all projects' tab (no filter), behavior is unchanged — still falls back to cwd resolution
- The daemon-side `project_abs_path` path is unchanged; only the project name passed to it is now correct

## Open Tail

- Should spawning from the 'all projects' tab be disallowed or prompt for a project instead of falling back to cwd?

## Evidence

- transcript lines 7-7
- transcript lines 154-162
- transcript lines 196-229


---
type: episode-card
date: 2026-06-15
session: 9c78b46a-3169-42eb-84c1-228a6c2f6589
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9c78b46a-3169-42eb-84c1-228a6c2f6589.jsonl
salience: product
status: active
subjects:
  - tmux-tui-spawn-project-resolution
supersedes:
  - 2026-06-15-1-tmux-tui-spawn-resolves-project-from
related_claims: []
source_lines:
  - 7-7
  - 154-165
  - 205-229
captured_at: 2026-06-15T08:18:15Z
---

# Episode: TUI spawn respects selected project tab instead of cwd

## Prior State

When spawning a new tmux session from the TUI's spawnable-agent handler, the project was always resolved from the TUI process's current working directory (std::env::current_dir()), ignoring which project tab the user had selected.

## Trigger

User reported that creating a new session in the TUI places it in the pwd instead of the selected project's directory — the daemon received the wrong project name derived from cwd, not from the active tab.

## Decision

Use the active project tab filter (pf) as the project for spawning, falling back to cwd resolution only on the 'all projects' tab where no single project is selected: `let project = pf.map(str::to_string).unwrap_or_else(|| crate::project::resolve(&std::env::current_dir().unwrap_or_default()));`

## Consequences

- Spawning from a specific project tab now correctly creates the session in that project's directory
- On the 'all projects' tab (no filter), spawning still falls back to cwd — may need future policy decision on whether to disallow or prompt

## Open Tail

- All-projects tab spawn still uses cwd fallback; user may want it to prompt for a project or be disallowed

## Evidence

- transcript lines 7-7
- transcript lines 154-165
- transcript lines 205-229


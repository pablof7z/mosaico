---
type: episode-card
date: 2026-06-15
session: 622711fa-5176-4580-b311-d66446c2924b
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/622711fa-5176-4580-b311-d66446c2924b.jsonl
salience: product
status: active
subjects:
  - tmux-tui-keybindings
  - tmux-tui-session-display
supersedes:
  - 2026-06-15-2-replace-n-spawn-key-with-enter
related_claims: []
source_lines:
  - 3-4
  - 797-851
captured_at: 2026-06-15T07:11:20Z
---

# Episode: TUI spawn key unified to Enter, [no tmux] tag removed

## Prior State

Spawning a new agent required pressing [n]; the [no tmux] tag was shown for sessions without a live tmux pane; hint bar listed [n] spawn separately

## Trigger

User corrections: (1) all local sessions are now resumable so [no tmux] tag is obsolete, (2) spawn should use Enter not [n]

## Decision

Enter key now handles both attach (on live sessions) and spawn (on spawnable items); [n] keybinding and all [n] hint text removed; [no tmux] tag removed from live session display lines; all live sessions render with same color styling regardless of attachability

## Consequences

- Simpler TUI key model: Enter is the single action key for attach-or-spawn
- Hint bar reads [↵] attach/spawn instead of [a/↵] attach [n] spawn
- Sessions that lack a tmux pane are no longer visually distinguished in the list

## Open Tail

*(none)*

## Evidence

- transcript lines 3-4
- transcript lines 797-851


---
type: episode-card
date: 2026-06-15
session: 622711fa-5176-4580-b311-d66446c2924b
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/622711fa-5176-4580-b311-d66446c2924b.jsonl
salience: product
status: superseded
subjects:
  - tenex-edge-tmux-tui
  - spawn-keybinding
supersedes: []
related_claims: []
source_lines:
  - 3-5
  - 54-56
  - 799-822
  - 846-847
captured_at: 2026-06-15T07:05:43Z
---

# Episode: Replace [n] spawn key with Enter in TUI

## Prior State

Spawning a new session in the TUI required pressing [n]; Enter was bound only to attach/resume actions on existing sessions.

## Trigger

User directive: new session creation should use Enter, not [n], for a more natural interaction.

## Decision

Enter now contextually handles both attach (on live/resumable sessions) and spawn (on spawnable agent rows). The [n] key binding and its hint text are removed; the footer now reads [↵] attach/spawn.

## Consequences

- Single key (Enter) drives all primary actions in the TUI
- [n] binding removed from KeyCode::Char('n') handler and hint bar
- "Press [n] to spawn." status message removed

## Open Tail

*(none)*

## Evidence

- transcript lines 3-5
- transcript lines 54-56
- transcript lines 799-822
- transcript lines 846-847


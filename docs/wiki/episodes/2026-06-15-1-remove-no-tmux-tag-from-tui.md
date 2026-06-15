---
type: episode-card
date: 2026-06-15
session: 622711fa-5176-4580-b311-d66446c2924b
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/622711fa-5176-4580-b311-d66446c2924b.jsonl
salience: product
status: active
subjects:
  - tenex-edge-tmux-tui
  - no-tmux-label
supersedes: []
related_claims: []
source_lines:
  - 3-5
  - 769-796
  - 844-845
captured_at: 2026-06-15T07:05:43Z
---

# Episode: Remove [no tmux] tag from TUI session list

## Prior State

Non-attachable sessions in the TUI session list displayed a dimmed [no tmux] tag, signaling that the session had no live tmux endpoint to attach to.

## Trigger

User directive: all local sessions are now resumable, making the [no tmux] indicator unnecessary.

## Decision

Removed the [no tmux] display branch entirely; all live sessions now render with consistent styling regardless of attachability.

## Consequences

- TUI session list no longer distinguishes attachable vs non-attachable sessions visually
- Resumable sessions make the tmux-attachment status irrelevant as a display concern

## Open Tail

*(none)*

## Evidence

- transcript lines 3-5
- transcript lines 769-796
- transcript lines 844-845


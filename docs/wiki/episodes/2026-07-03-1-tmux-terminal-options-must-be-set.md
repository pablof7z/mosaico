---
type: episode-card
date: 2026-07-03
session: abce9e9f-8f3e-4561-9dd3-684afd59be80
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/abce9e9f-8f3e-4561-9dd3-684afd59be80.jsonl
salience: root-cause
status: active
subjects:
  - tmux-launch
  - terminal-options
  - color-support
  - term-env
supersedes: []
related_claims: []
source_lines:
  - 1-1
  - 395-430
  - 458-490
  - 1003-1019
  - 1242-1269
captured_at: 2026-07-03T10:34:00Z
---

# Episode: tmux terminal options must be set globally before session fork, not per-session after

## Prior State

make_session_transparent() set default-terminal and terminal-overrides via `tmux set-option -t <session>` AFTER `tmux new-session -d` had already forked the child agent process. The terminal-overrides value was also malformed: `,*:Tc,RGB,extkeys` had orphaned capability tokens with no term-pattern prefix.

## Trigger

User reported that launching any agent harness via `tenex-edge launch` produced no colors, while launching the same harness directly worked fine. Screenshots confirmed the regression was new as of that day.

## Decision

Introduced `ensure_global_terminal_options()` that sets `default-terminal tmux-256color` and `terminal-overrides *:Tc:RGB:extkeys` via `tmux set-option -g` BEFORE `new-session` forks the harness child. Removed the per-session `set-option -t` calls for these two options from `make_session_transparent()`. Also corrected the terminal-overrides format from `,*:Tc,RGB,extkeys` to `*:Tc:RGB:extkeys`.

## Consequences

- Child agent processes now inherit the correct $TERM (tmux-256color) and $COLORTERM (truecolor) at fork time, restoring ANSI 256-color and truecolor rendering inside tmux-spawned panes.
- The terminal options are now a server-wide global invariant set before any session spawn, not a per-session override — any future code that relies on per-session terminal overrides will not work and must use the global pre-fork path instead.
- The malformed terminal-overrides string is now a historical artifact; the corrected format uses colon-separated capabilities under a single term-pattern prefix.

## Open Tail

- The fix is on branch fix-launch-color-tesession in a worktree, uncommitted — user has not yet decided whether to commit or restart the production daemon.

## Evidence

- transcript lines 1-1
- transcript lines 395-430
- transcript lines 458-490
- transcript lines 1003-1019
- transcript lines 1242-1269

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-03-1-tmux-terminal-options-must-be-set.json`](transcripts/2026-07-03-1-tmux-terminal-options-must-be-set.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-03-1-tmux-terminal-options-must-be-set.json`](transcripts/raw/2026-07-03-1-tmux-terminal-options-must-be-set.json)

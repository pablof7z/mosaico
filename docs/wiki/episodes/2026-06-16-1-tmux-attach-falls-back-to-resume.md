---
type: episode-card
date: 2026-06-16
session: a7c75cc2-efc0-47db-aa7d-9332d6c63310
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a7c75cc2-efc0-47db-aa7d-9332d6c63310.jsonl
salience: product
status: active
subjects:
  - tmux-attach-fallback
  - tui-session-resume
supersedes: []
related_claims: []
source_lines:
  - 1-9
  - 94-137
captured_at: 2026-06-16T11:56:17Z
---

# Episode: tmux attach falls back to resume on stale pane

## Prior State

When a tmux pane was no longer live (stale %pane_id), the TUI surfaced a dead-end error like 'Attach failed: pane %110 not found' or 'Session pane not found.' — no recovery path for the user.

## Trigger

User directive (line 1): 'this error should never exist — if the tmux pane is not attachable then we just resume the session as if it weren't attached to a tmux… that's it…'

## Decision

Changed PendingAttach from a bare Option<String> (pane id) to a struct carrying both pane id and a resume_sid fallback. When attach to a pane fails, the TUI transparently resumes the session via the daemon and attaches to the fresh pane — exactly as if the session had never been in tmux. Only surfaces an error if resume itself also fails.

## Consequences

- All four pending_attach sites updated: live-attach, spawn, Enter-resume, r-resume
- Spawn path gets resume_sid = None (nothing to fall back to, since freshly spawned)
- The 'pane not found' / 'Attach failed' error class is eliminated from user-facing TUI
- Attach is now best-effort with automatic transparent fallback

## Open Tail

*(none)*

## Evidence

- transcript lines 1-9
- transcript lines 94-137


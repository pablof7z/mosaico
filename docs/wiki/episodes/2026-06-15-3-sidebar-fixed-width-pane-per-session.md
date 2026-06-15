---
type: episode-card
date: 2026-06-15
session: 9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf.jsonl
salience: product
status: active
subjects:
  - tenex-edge-tmux-sidebar
  - session-switching
supersedes: []
related_claims: []
source_lines:
  - 89-113
  - 616-618
  - 629-635
  - 915-916
  - 1082-1084
  - 1148-1149
captured_at: 2026-06-15T08:15:14Z
---

# Episode: Sidebar: fixed-width pane-per-session with resize hook

## Prior State

No sidebar existed. Three options were evaluated: A (persistent split-pane), B (popup on keypress), C (tmux built-in chooser). Initial sidebar build used -l 32 initial width, but tmux rescales panes proportionally on client resize — sidebar became huge on small windows or tiny on large ones.

## Trigger

User requested always-visible sidebar (Option A) at line 616; then reported sidebar 'can be huge or too small depending on the window size' and asked for fixed width (line 915).

## Decision

Sidebar lives as a fixed 40-col left pane inside each agent's own window (forced by tmux's one-session-per-client model + one-independent-session-per-agent architecture). Width is pinned via client-resized hook (resize-pane -x 40) that re-fixes it on every resize. ensure_sidebar injects the pane idempotently on attach; switching sessions also injects a sidebar into the target before switch-client.

## Consequences

- Pane-per-session model: each agent window gets its own sidebar process, not a shared global pane
- Switching sessions requires ensure_sidebar on the target before switch-client so the sidebar persists across switches
- client-resized hook is session-scoped and dies with the session; no recursion since resize-pane fires window-layout-changed not client-resized
- Alt-s focuses sidebar, Alt-a focuses agent pane; sidebar highlights current session with ► in cyan bold
- Sidebar auto-appears on both attach paths (TUI Enter key and CLI attach --session)

## Open Tail

- Sidebar and popup both launch tenex-edge by bare name — needs $PATH resolution or current_exe() fallback for dev builds

## Evidence

- transcript lines 89-113
- transcript lines 616-618
- transcript lines 629-635
- transcript lines 915-916
- transcript lines 1082-1084
- transcript lines 1148-1149


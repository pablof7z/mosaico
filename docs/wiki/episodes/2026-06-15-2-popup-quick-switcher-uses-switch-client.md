---
type: episode-card
date: 2026-06-15
session: 9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf.jsonl
salience: product
status: active
subjects:
  - tenex-edge-tmux-popup
  - session-switching
supersedes:
  - 2026-06-15-2-always-visible-session-switcher-sidebar-adopted
related_claims: []
source_lines:
  - 929-931
  - 965-970
  - 1028-1031
  - 1144-1147
captured_at: 2026-06-15T08:15:14Z
---

# Episode: Popup quick-switcher uses switch-client not inline attach

## Prior State

Popup (display-popup -E) ran the full TUI inline inside the overlay; selecting a session attached inline within the popup, trapping the session and its sidebar inside the floating overlay instead of moving the real tmux client.

## Trigger

Assistant diagnosed that display-popup runs the TUI in-process, so attach flows execute inside the popup rather than switching the underlying client (line 929). User confirmed wanting to keep the popup feature while building the sidebar.

## Decision

Add --popup flag to bare tmux command. In popup mode, all session-selection paths (attach/spawn/resume) do switch-client on the underlying client + ensure_sidebar on the target session, then exit — which closes the -E popup and hands the full terminal to the chosen session.

## Consequences

- M-t binding injected permanently via bind_sidebar_keys alongside Alt-s/Alt-a
- Popup launches tenex-edge tmux --popup, not the bare command
- All three selection paths (attach, spawn, resume) funnel through the same switch-client+exit logic in popup mode
- Popup stays intentionally relative-sized (80%×80%) for readability across terminal sizes

## Open Tail

- Both popup and sidebar launch tenex-edge by bare name — requires it on $PATH; could switch to std::env::current_exe() for cargo run dev builds

## Evidence

- transcript lines 929-931
- transcript lines 965-970
- transcript lines 1028-1031
- transcript lines 1144-1147


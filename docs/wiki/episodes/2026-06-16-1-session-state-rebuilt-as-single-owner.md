---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: architecture
status: superseded
subjects:
  - session-state
  - session-identity
  - daemon-architecture
  - status-outbox
supersedes:
  - 2026-06-16-1-session-state-single-owner-aggregate-replaces
related_claims: []
source_lines:
  - 1-27
  - 1014-1028
  - 1037-1038
  - 1091-1145
  - 1214-1253
  - 1256-1258
  - 1383-1384
  - 1412-1431
  - 1432-1440
  - 1487-1505
captured_at: 2026-06-16T11:59:49Z
---

# Episode: Session state rebuilt as single-owner aggregate with daemon-minted identity

## Prior State

Session state (title, activity, busy, liveness) scattered across ≥4 stores (task-local cur_title/cur_activity, session_status table, legacy agent_status table, kind:30315 relay tag) written from ~7 scattered call sites with no single apply(transition) chokepoint. CQRS inverted: volatile task frame was the write model, sqlite was a lossy mirror. Session identity (session_id) was borrowed from harnesses with unstable origin — opencode mints a fresh ID every start, creating unbounded competing permanent title events on the relay. Three live bugs stemmed from this: title one turn behind (prompt not seeded at turn-start), title never generated (30s turn_first gate exceeded most turn durations), flip-flop between two titles (race in spawn_session).

## Trigger

User frustration with cascading title/status bugs ('everything fucking sucks') prompted dispatching two independent architectural research agents (Opus + Codex) against the same brief. Both converged on the same diagnosis: no single owner of session state + borrowed identity = drift by construction.

## Decision

Full session aggregate refactor, no backwards compatibility: daemon-minted stable session_key as the only identity (harness native IDs become aliases in session_aliases table); single session_state row as source of truth with explicit transition methods (start_turn, seed_title_if_empty, apply_distill_result with version guard, heartbeat, end, supersede); one deterministic derive_status() projection shared by who/statusline/delta; stateless runtime driver (on_tick → effects); status_outbox table for versioned publication; delete legacy agent_status/session_status outright. Three interim tactical fixes (atomic spawn reservation, prompt-seed at turn-start, turn_first 30s→3s) applied immediately and subsumed by the aggregate later.

## Consequences

- Opencode per-start session ID churn resolves to one stable key, making competing orphaned relay events structurally impossible
- Runtime task death/respawn loses zero state — cur_title/cur_activity removed from task locals, read from session_state row instead
- Stale distill results structurally cannot flip the title (apply_distill_result no-ops if turn_id + state_version no longer match)
- who local-vs-peer fork eliminated — both branches route through derive_status()
- Delta/incremental update contract preserved: full roster on turn 1, only changes (status transitions, new sessions, expired sessions, arrived chat) on subsequent turns, using updated_at/first_seen cursors on the new tables
- Dual-write to legacy canonical tables deferred to a follow-up PR (different subsystem, not the title bug source)

## Open Tail

- Implementation in progress on session-state-rearchitecture branch via fanned-out agents (foundation → parallel migration → integration)
- Messages/inbox/chat and project/membership canonicalization not in this PR
- Build-to-green and integration test verification still pending

## Evidence

- transcript lines 1-27
- transcript lines 1014-1028
- transcript lines 1037-1038
- transcript lines 1091-1145
- transcript lines 1214-1253
- transcript lines 1256-1258
- transcript lines 1383-1384
- transcript lines 1412-1431
- transcript lines 1432-1440
- transcript lines 1487-1505


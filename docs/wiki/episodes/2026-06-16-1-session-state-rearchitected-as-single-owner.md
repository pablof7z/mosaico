---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: architecture
status: superseded
subjects:
  - session-state-aggregate
  - session-identity
  - derive-status
supersedes:
  - 2026-06-16-3-session-state-as-single-owner-aggregate
related_claims: []
source_lines:
  - 1037-1038
  - 1091-1146
  - 1214-1253
  - 1384-1431
  - 1440-1504
captured_at: 2026-06-16T12:09:16Z
---

# Episode: Session state rearchitected as single-owner aggregate

## Prior State

Session state (title, activity, busy, liveness) was split across ≥4 stores (task-local cur_title/cur_activity, session_status table, legacy agent_status table, kind:30315 relay tag), written from ~7 scattered call sites with no single apply(transition) chokepoint. session_id was borrowed from the harness (claude/codex adopt their native id, opencode mints a fresh id every start) rather than daemon-minted, so identity churn × permanent relay events = unbounded competing title events. CQRS was inverted: the volatile task frame was the write model and sqlite was a lossy mirror.

## Trigger

User frustration with cascading session-state bugs (title lagging one message, title never generated, flip-flop between two titles) led to commissioning two independent research agents (Opus and Codex). Both converged on the same structural root causes: no single owner of session state, and identity borrowed not minted. User chose 'full session aggregate' scope and mandated no backwards compatibility.

## Decision

Adopt daemon-minted stable session_key as the only identity (harness ids become aliases in a session_aliases table). One session_state row (keyed by session_key) is the single source of truth, mutated only through explicit transition methods (register_or_reassert_session, start_turn, seed_title_if_empty, apply_distill_result, heartbeat, end, supersede). Runtime task becomes a stateless SessionDriver whose on_tick returns Vec<Effect>. One deterministic derive_status(state, now) projection shared by who (both branches), statusline, and turn delta. Status publication flows through a status_outbox table with a drainer rather than inline publish. Peer state split into separate peer_session_state that the materializer physically cannot write local state to. Delta contract preserved: full roster on first turn, only changes thereafter (new sessions, status transitions, expired sessions, arrived chat).

## Consequences

- Title-drift / orphaned-event / status-flip class of bugs become structurally unrepresentable rather than patched
- opencode per-start id churn resolves to one stable session_key; competing orphaned events become impossible
- Legacy agent_status and session_status tables deleted outright (no backcompat)
- Three interim tactical fixes (atomic spawn, prompt-seed, distill-timing 30s→3s) are consistent with the plan; atomic spawn explicitly kept as belt-and-suspenders
- All delta/query readers (who, statusline, turn-context) must go through derive_status; the local-vs-peer fork in who.rs is eliminated
- Session state survives task death/duplication; cur_title/cur_activity locals removed from runtime
- Migration sequenced in 8 incremental test-first phases starting with freeze invariants

## Open Tail

- Foundation + migration agents launched on session-state-rearchitecture branch; integration (build-to-green + test suite) still in progress
- Messages/inbox/chat and project/membership canonicalization deferred to a separate follow-up PR (different subsystem, not the source of the title bugs)

## Evidence

- transcript lines 1037-1038
- transcript lines 1091-1146
- transcript lines 1214-1253
- transcript lines 1384-1431
- transcript lines 1440-1504


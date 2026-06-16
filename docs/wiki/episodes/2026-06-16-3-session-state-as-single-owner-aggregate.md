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
  - daemon-architecture
supersedes:
  - 2026-06-16-1-session-state-rebuilt-as-single-owner
related_claims: []
source_lines:
  - 1037-1431
captured_at: 2026-06-16T12:05:20Z
---

# Episode: Session state as single-owner aggregate with minted identity

## Prior State

Session state had no single owner — one logical fact (session S is titled T, doing A, busy B) physically lived in ≥4 stores (task-local cur_title, session_status table, legacy agent_status table, kind:30315 tag), written from ~7 scattered call sites with no apply(transition) chokepoint. CQRS was inverted: the volatile task frame was the write model and sqlite was a lossy mirror. Session identity was borrowed from harnesses (claude/codex adopted their native id, opencode minted a fresh id every start, the daemon generated te-* otherwise) — combined with kind:30315 never expiring, identity rotation × permanent events = unbounded competing title events by construction.

## Trigger

User demanded architectural research after three tactical bug fixes still felt like 'everything is stitched together and state is all over the place.' Two independent research agents (Opus + Codex) converged on the same diagnosis: no session aggregate, no minted identity.

## Decision

Full session aggregate refactor, no backwards compatibility: (1) Daemon-minted stable session_key is the only identity; harness ids become aliases in a session_aliases table. (2) One session_state row (session_key PK; title, title_source, activity, phase, turn_id, state_version, lifecycle, etc.) as single source of truth, mutated only through explicit transition methods (start_turn, seed_title_if_empty, apply_distill_result, heartbeat, end, supersede). (3) Stateless SessionDriver whose on_tick returns Vec<Effect>, table-testable. (4) One deterministic derive_status() projection shared by who, statusline, and delta. (5) status_outbox table for versioned publication with retry. (6) peer_session_state separates local from peer state. (7) Legacy agent_status/session_status tables deleted outright.

## Consequences

- Migration sequenced as incremental phases (freeze tests → identity → session_state+transitions → derive_status → relay lifecycle → legacy cleanup)
- All status writes flow through one transaction that bumps state_version and enqueues an outbox row
- Runtime task loses cur_title/cur_activity and never builds DomainEvent::Status directly
- Delta contract preserved: full roster on turn 1, only changes thereafter, project-scoped and self-excluded
- No backwards compat — legacy tables deleted, old d-tag addresses age out

## Open Tail

- messages/inbox/chat and project/membership canonicalization deferred to a follow-up PR

## Evidence

- transcript lines 1037-1431


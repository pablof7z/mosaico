---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: architecture
status: superseded
subjects:
  - session-state
  - session-key
  - derive-status
  - status-outbox
  - session-aliases
  - delta-rendering
supersedes:
  - 2026-06-16-2-full-session-aggregate-architecture-adopted
related_claims: []
source_lines:
  - 1037-1038
  - 1092-1145
  - 1167-1191
  - 1214-1252
  - 1346-1362
  - 1412-1503
captured_at: 2026-06-16T11:42:46Z
---

# Episode: Full session-state aggregate architecture adopted

## Prior State

Session state (title, activity, busy, liveness) scattered across ≥4 stores with no single owner — runtime task-local cur_title/cur_activity, session_status table, legacy agent_status table, and the kind:30315 relay tag — written from ~7 scattered call sites with no transition chokepoint. CQRS inverted: the volatile task frame was the write model and sqlite was a lossy mirror. Session identity was borrowed from the harness (opencode mints a fresh id every start), and combined with permanent relay events produced unbounded competing title broadcasts. Local vs peer status derived via two different code paths with no shared projection.

## Trigger

User directive: 'everything feels incredibly buggy and stitched together — rethink the architecture'; two independent research agents (Opus Architect, Codex) converged on the same root cause and cure

## Decision

Adopt full session aggregate: daemon-minted stable session_key as the only identity (harness IDs become aliases in session_aliases table); one session_state row (PK=session_key, columns including title, title_source, activity, phase, turn_id, turn_started_at, last_distill_at, last_seen, state_version, lifecycle) as single source of truth mutated only through explicit transition methods (start_turn, seed_title_if_empty, apply_distill_result, heartbeat, end); stateless runtime driver (on_tick → Vec<Effect>); derive_status(now) shared by who/statusline/delta; status_outbox for relay publication with retry; delta support (full roster on first turn, changes-only thereafter) built into schema cursors (updated_at, first_seen)

## Consequences

- Bad state becomes structurally unrepresentable rather than merely unreached
- Kills the local-vs-peer fork in who.rs
- Opencode per-start id churn resolves to one stable key via aliases
- Runtime task death/respawn loses zero state
- Versioned distill application ignores stale results structurally
- Delta rendering preserved: full roster turn 1, only changes thereafter
- 8-phase incremental migration with freeze invariants written as failing tests first
- Legacy agent_status and session_status tables deleted outright (no backwards compatibility)

## Open Tail

- Phase 0 freeze tests not yet written
- messages/inbox/chat and project/membership canonicalization deferred to a follow-up PR
- Workflow agents executing on branch session-state-rearchitecture

## Evidence

- transcript lines 1037-1038
- transcript lines 1092-1145
- transcript lines 1167-1191
- transcript lines 1214-1252
- transcript lines 1346-1362
- transcript lines 1412-1503


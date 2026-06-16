---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: architecture
status: active
subjects:
  - session-state-aggregate
  - session-identity
  - daemon-architecture
supersedes:
  - 2026-06-16-1-session-state-rearchitected-as-single-owner
related_claims: []
source_lines:
  - 1087-1126
  - 1214-1237
  - 1262-1268
  - 1383-1384
captured_at: 2026-06-16T12:33:38Z
---

# Episode: Session state redesign: single-owner aggregate with minted identity

## Prior State

Session state was split across ≥4 stores (task-local cur_title, session_status, legacy agent_status, kind:30315 d-tag), written from ~7 scattered call sites with no single transaction or transition chokepoint. session_id was borrowed from the harness (unstable across restarts; opencode mints a new one each start), simultaneously used as sqlite PK, relay d-tag, and routing target. CQRS was inverted: the volatile task frame was the write model and sqlite a lossy mirror.

## Trigger

Two independent LLM research runs (Opus + Codex) converged on the same diagnosis: (1) no single owner for session state, and (2) identity is borrowed not minted. Every recurring title/status bug traced to these two root causes. User then explicitly chose 'Full session aggregate' scope and 'no backwards compatibility'.

## Decision

Replace the scattered status tables and task-local state with a single session_state row as source of truth (keyed by a daemon-minted stable session_key), with session_aliases mapping harness ids. All mutation flows through explicit transition methods (start_turn, seed_title, apply_distill_result, heartbeat, end, supersede) that each run in one sqlite transaction, bump state_version, and enqueue a status_outbox row. Runtime task becomes a stateless controller; derive_status() is a deterministic projection shared by who/statusline/publisher. Legacy agent_status and session_status tables are deleted outright.

## Consequences

- Daemon owns session identity — harness id churn is hidden behind alias resolution
- One row per logical session; title drift across stores is structurally impossible
- Status relay event is a projection of the row, not a competing source
- Versioned distill: stale apply_distill_result no-ops if turn_id/state_version mismatch
- status_outbox + drainer: publication is decoupled from mutation, retries are structural
- Delta contract preserved: full roster on first turn, only changes thereafter
- 6-phase incremental migration (Phase 0 = freeze invariants as failing tests first)

## Open Tail

- messages/inbox/chat canonicalization and project/membership are a separate subsystem, not addressed this PR
- peer_session_state materializer must write to peer_state not session_state (Phase 5 of plan)

## Evidence

- transcript lines 1087-1126
- transcript lines 1214-1237
- transcript lines 1262-1268
- transcript lines 1383-1384


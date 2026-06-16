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
  - session-aliases
  - status-outbox
supersedes:
  - 2026-06-16-3-full-session-state-aggregate-architecture-adopted
related_claims: []
source_lines:
  - 1037-1037
  - 1096-1146
  - 1214-1252
  - 1383-1422
  - 1432-1439
  - 1497-1504
captured_at: 2026-06-16T11:54:39Z
---

# Episode: Session state: single-owner aggregate replaces scattered multi-store writes

## Prior State

Session state had no single owner — one logical fact (session S is titled T, doing A, busy B) lived in ≥4 stores (runtime task-local cur_title/cur_activity, session_status table, legacy agent_status table, kind:30315 tag), written from ~7 scattered call sites with no apply(transition) chokepoint. Session identity was borrowed from the harness (opencode mints a fresh id every start), not minted by the daemon, causing orphaned competing title events by construction.

## Trigger

Three live bugs diagnosed in this session — title one-turn-behind (seed read transcript before Claude flushed), distiller never firing (turn_first=30s > turn duration), flip-flop dual sessions (non-atomic spawn) — plus the user's explicit directive: 'everything feels incredibly buggy and stitched together like state is all over the place — take a step back and rethink the architecture.' Two independent research agents (Opus, Codex) converged on the same root cause.

## Decision

Daemon-minted stable session_key as the only identity (harness ids become aliases in session_aliases table). One session_state row (PK=session_key) as single source of truth, mutated only through explicit transition methods (register_or_reassert_session, start_turn, seed_title_if_empty, apply_distill_result, heartbeat, end, supersede), each a single sqlite transaction bumping state_version and enqueuing a status_outbox row. Runtime becomes a stateless SessionDriver (on_tick → Vec<Effect>). Legacy agent_status and session_status tables deleted outright. Delta contract preserved via updated_at/first_seen cursors on the new tables.

## Consequences

- Identity churn (opencode per-start ids) resolves to one stable key; competing orphaned events become structurally impossible
- Title drift / status flip-flop / orphaned events become unrepresentable rather than merely patched
- All ~7 scattered set_agent_status call sites replaced by one commit_session_state transition path
- Versioned distill application: stale distill results no-op if turn_id or state_version no longer match
- DaemonState.last_status bug (keyed by (pubkey, project) — wrong for multi-session) subsumed by per-session outbox
- Three tactical fixes from this session (atomic spawn, prompt-seed, distill-timing) are interim hardening consistent with the plan; atomic spawn is explicitly 'keep' as belt-and-suspenders
- Migration: 8 incremental phases starting with freeze tests as failing-first oracles

## Open Tail

- Phase 0 freeze tests not yet written
- Messages/inbox/chat and project/membership canonicalization deferred to follow-up PR
- Pure on_tick vs controller-calls-transitions stylistic fork still open (plan recommends pure on_tick for driver + outbox for publish)

## Evidence

- transcript lines 1037-1037
- transcript lines 1096-1146
- transcript lines 1214-1252
- transcript lines 1383-1422
- transcript lines 1432-1439
- transcript lines 1497-1504


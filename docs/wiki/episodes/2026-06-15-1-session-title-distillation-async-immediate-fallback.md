---
type: episode-card
date: 2026-06-15
session: 215d979a-a054-4e2b-b349-851e0d874d6d
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/215d979a-a054-4e2b-b349-851e0d874d6d.jsonl
salience: root-cause
status: superseded
subjects:
  - session-title-distillation
  - engine-loop
  - distill-timeout
supersedes:
  - 2026-06-14-2-title-distillation-engine-stalls-on-slow
related_claims: []
source_lines:
  - 137-1220
captured_at: 2026-06-15T07:32:05Z
---

# Episode: Session title distillation: async + immediate fallback + retry-on-failure

## Prior State

Session title distillation was awaited synchronously inside the engine's `select!` loop, blocking all heartbeats, presence refreshes, and turn-end detection for the full duration of the OpenRouter API call (6+ minutes observed). `last_distill` was set unconditionally even when `distill_session` returned `None`, consuming the single distillation slot per turn with no retry. No title existed until the LLM call returned, leaving the TUI showing bare 'working' for 15+ minutes.

## Trigger

User observed this session had no published title for 15 minutes. Root-cause analysis found two bugs: (1) `complete_via_rig` has no timeout — blocks the engine indefinitely; (2) `last_distill = now` executes on failure, permanently eating the one distillation chance per turn.

## Decision

Three architectural changes to the engine loop: (1) On turn rising edge, if `cur_title` is None, immediately set title from the last user prompt (titleized, truncated) and publish it before any LLM call. (2) Distillation is now a spawned background task with a 20s timeout; the `obs.tick()` arm checks `is_finished()` non-blockingly, keeping the engine fully responsive. (3) `last_distill` is only written on success; a separate `last_distill_attempt` timestamp gates retries so failures get another `turn_first` window instead of being silently skipped.

## Consequences

- Sessions always show a meaningful title immediately on first user message (user-prompt fallback), even if distillation is slow or fails
- Engine loop never freezes — heartbeats, presence, and turn-end detection continue during distillation
- Failed distillation attempts are retried after another `turn_first` window rather than permanently lost
- The 20s timeout bounds worst-case latency for any single distillation call

## Open Tail

- Observation needed: does the immediate prompt-derived title cause flicker when the LLM-distilled title replaces it shortly after?
- Retry count is unbounded — a persistently failing distillation endpoint will retry every `turn_first` interval indefinitely

## Evidence

- transcript lines 137-1220


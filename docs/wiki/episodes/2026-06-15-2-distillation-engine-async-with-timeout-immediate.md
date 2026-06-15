---
type: episode-card
date: 2026-06-15
session: 215d979a-a054-4e2b-b349-851e0d874d6d
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/215d979a-a054-4e2b-b349-851e0d874d6d.jsonl
salience: architecture
status: active
subjects:
  - session-title-distillation
  - engine-obs-loop
  - transcript-reader
supersedes:
  - 2026-06-15-1-session-title-distillation-async-immediate-fallback
related_claims: []
source_lines:
  - 137-1270
captured_at: 2026-06-15T08:02:57Z
---

# Episode: Distillation engine: async with timeout, immediate prompt-seeded title, retry on failure

## Prior State

Distillation was awaited synchronously inside the tokio::select! obs.tick() arm, blocking all heartbeats/presence for the full API call duration (6+ minutes observed). No timeout existed. last_distill was set unconditionally even when distill_session returned None, consuming the single retry slot per turn (turn_repeat=0). No title existed until the LLM call completed, leaving sessions titleless for 30s+ after turn start.

## Trigger

User discovered session had no visible title for 15 minutes. Root cause analysis identified: (1) complete_via_rig blocks the engine indefinitely on slow OpenRouter responses, (2) last_distill = now on failure prevents retries, (3) no fallback title before LLM completes. User explicitly requested: immediate title from user message, 20s async timeout with retries, and fix the unconditional last_distill update.

## Decision

Three changes: (1) On rising edge when cur_title is None, immediately read the last user prompt via new transcript::read_last_user_prompt and titleize it (truncate at 60 chars at word boundary) — publish before any LLM call. (2) Spawn distill_session in a background tokio::task wrapped in tokio::time::timeout(20s); poll is_finished() each obs.tick() so the engine never blocks. (3) Separate last_distill (only set on Some(labels) success) from last_distill_attempt (set on every spawn); after failure, retry after another turn_first window.

## Consequences

- Engine loop remains fully responsive even when OpenRouter is slow or down
- Sessions always have a visible title from the moment the first user prompt is processed
- Distillation failures no longer permanently consume the retry slot; the engine retries after another turn_first interval
- Falling edge (turn end) resets last_distill_attempt and distill_task so next turn starts fresh
- Stale distill results (from a prior turn) are discarded via distill_task_turn comparison

## Open Tail

*(none)*

## Evidence

- transcript lines 137-1270


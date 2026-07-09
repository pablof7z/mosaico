---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: product
status: active
subjects:
  - distill-failure-notice
  - session-distill-streak
  - turn-context-warnings
supersedes:
  - 2026-07-09-a62822c5d09c-fbf565a0-4-distillation-silent-failure-root-cause-and
related_claims: []
source_lines:
  - 1211-1232
  - 1262-1262
  - 1405-1449
  - 1809-1833
  - 1891-1896
  - 1934-1938
captured_at: 2026-07-09T14:50:00Z
---

# Episode: Distill failure surfacing: throttled agent-facing warning replaces silent swallow

## Prior State

Distill failures were completely invisible: the only handler was a debug `slog` line in `runtime.rs`. The prior `session_errors` table and `distill_error` statusline flash were deleted in a schema rewrite and never replaced. The `distill_error` field in `statusline.rs` is dead code always set to `None`. A persistently-failing distiller (e.g. dead Ollama endpoint) left session titles blank forever with no signal to agents or users.

## Trigger

User discovered live, ongoing distill failures: Ollama was configured to port 8081 (squatted by Docker Desktop) instead of 11434. Every distill call failed silently for all sessions. User directed: fix the Ollama config, and patch the code so agents are told (in non-internal language) that status/title generation is failing, throttled to a few times per hour.

## Decision

Added `distill_fail_streak` and `distill_notice_at` columns to `sessions` (additive migration via `session_distill_notice.rs`). `runtime.rs` now calls `record_distill_failure` on each error, incrementing the streak. `assemble_turn_start` injects a `<warnings>` entry — phrased without the word 'distillation' — once 3 consecutive failures accumulate, throttled to at most once per 15 minutes while the streak persists. A successful distill resets the streak to 0.

## Consequences

- Agents now receive a throttled, user-friendly warning in their turn-start context when status/title generation has been persistently failing, enabling them to alert the user.
- The streak/notice state is persisted in the sessions table, surviving across turns and daemon restarts.
- Success resets the streak, so recovered distillers don't keep re-firing the warning.
- 3 regression tests cover: below-threshold silence, first-fire + throttle suppression, and streak-reset-on-success.
- The `distill_error` statusline field remains dead code (not wired).

## Open Tail

- The new failure-tracking code is compiled but not yet running on the live daemon — requires a daemon rebuild + restart, which would interrupt all live sessions on the machine.
- The `tenex-edge who` root cause is identified (session `seen_cursor` gates the agent roster, showing only 2/14 agents for deep sessions) but not yet fixed.

## Evidence

- transcript lines 1211-1232
- transcript lines 1262-1262
- transcript lines 1405-1449
- transcript lines 1809-1833
- transcript lines 1891-1896
- transcript lines 1934-1938

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-4-distill-failure-surfacing-throttled-agent-facing.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-4-distill-failure-surfacing-throttled-agent-facing.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-4-distill-failure-surfacing-throttled-agent-facing.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-4-distill-failure-surfacing-throttled-agent-facing.json)

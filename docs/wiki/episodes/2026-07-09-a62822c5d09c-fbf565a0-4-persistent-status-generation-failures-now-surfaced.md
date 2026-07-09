---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: product
status: active
subjects:
  - distill-failure-surfacing
  - turn-start-warnings
  - session-schema
  - agent-notification
supersedes:
  - 2026-07-09-a62822c5d09c-fbf565a0-4-distill-failure-surfacing-throttled-agent-facing
related_claims: []
source_lines:
  - 1117-1118
  - 1120-1232
  - 1262-1262
  - 1405-1449
  - 1936-1940
captured_at: 2026-07-09T18:32:48Z
---

# Episode: Persistent status-generation failures now surfaced to agents via throttled turn-start warning

## Prior State

Distill failures were completely invisible: the sole handler was a per-session debug log line (`slog` in `runtime.rs:281-283`) with no persistence, no counting, no warning to agents or users. The old `session_errors` table and `distill_error` statusline flash had been deleted in a prior schema rewrite and never replaced. Status titles stayed empty forever with no visible error anywhere except a per-session debug log file. The prior theory (30s debounce + no retry) was wrong — the distiller was running on schedule but failing every call because the Ollama endpoint was misconfigured to a Docker-squatted port.

## Trigger

User demanded urgent analysis of empty session titles. Root cause found: (1) operational — `OLLAMA_HOST` in `~/.zshrc` and `providers.json` both pointed at port 8081 (squatted by Docker Desktop) instead of Ollama's actual port 11434; (2) code bug — this failure mode was completely silent. User then directed: failing status generation should notify agents (without using the word 'distillation'), throttled to a few times per hour so agents don't pester the user.

## Decision

Added `distill_fail_streak` and `distill_notice_at` columns to the `sessions` table via additive migration (no wipe). `record_distill_failure` increments the streak on each failure in `runtime.rs`'s observe loop. `assemble_turn_start` injects a `<warnings>` entry after 3 consecutive failures, throttled to at most once per 15 minutes while the streak persists, using agent-facing language: 'This session's status/title updates haven't been generating successfully for a while. You may want to let the user know status updates aren't working right now.' A successful distill resets the streak to 0. Also fixed the Ollama endpoint config (both `~/.zshrc` and `providers.json` updated to port 11434, verified live).

## Consequences

- Agents now receive a turn-start warning when status generation has been failing persistently, enabling them to alert the user
- Throttling (3-failure threshold + 15-min cooldown) prevents agent pestering during ongoing outages
- Streak resets on success so warnings don't recur after recovery
- New schema columns are additive — existing on-disk DBs gain them without a wipe-and-rebuild
- The word 'distillation' never appears in agent-facing text
- 3 regression tests cover: below-threshold silence, first-fire + throttle suppression, streak-reset-on-success
- Code is compiled and tested (899/899) but not yet deployed — requires daemon restart which would interrupt all live sessions

## Open Tail

- Daemon restart pending to activate the throttled-notice code in the live process
- `tenex-edge who` roster-cursor bug identified (session's `seen_cursor` gates agent roster, so deep sessions only see recently-republished agents) but not yet fixed

## Evidence

- transcript lines 1117-1118
- transcript lines 1120-1232
- transcript lines 1262-1262
- transcript lines 1405-1449
- transcript lines 1936-1940

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-4-persistent-status-generation-failures-now-surfaced.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-4-persistent-status-generation-failures-now-surfaced.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-4-persistent-status-generation-failures-now-surfaced.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-4-persistent-status-generation-failures-now-surfaced.json)

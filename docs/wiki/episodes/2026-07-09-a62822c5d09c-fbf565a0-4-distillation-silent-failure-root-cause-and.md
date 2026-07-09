---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: root-cause
status: superseded
subjects:
  - distillation
  - ollama-config
  - silent-failure
  - title-generation
  - status-publish
supersedes: []
related_claims: []
source_lines:
  - 1117-1118
  - 1120-1232
  - 1262-1262
  - 1283-1317
  - 1384-1401
captured_at: 2026-07-09T14:42:06Z
---

# Episode: Distillation silent-failure root cause and agent-facing notice directive

## Prior State

Empty titles on kind:30315 status events were theorized to be a debounce/timing issue — `turn_first` defaults to 30s, `turn_repeat` defaults to disabled (0), and first distill attempts might fail silently with no retry. The distillation code silently falls back to `current_title` (which is `None` for fresh sessions) on any LLM error, leaving titles blank forever with no visible error in daemon.log or on the wire.

## Trigger

User demanded real-time analysis of the empty-title bug. Per-session log investigation revealed the distiller was running on schedule every ~30s but failing every single time: the `edge-distillation` role pointed at Ollama `http://localhost:8081`, but port 8081 was squatted by Docker Desktop and no Ollama process was running. The actual Ollama instance was on port 11434. Errors were only visible in per-session debug logs, not in daemon.log or any surfaced status.

## Decision

(1) Operational fix: corrected `OLLAMA_HOST` in `~/.zshrc` and `ollama.apiKey` in `~/.tenex-edge/providers.json` from port 8081 to 11434; confirmed distillation working live. (2) Product directive: when distillation persistently fails, inject a notice into agent context (using non-internal terminology, not 'distillation') so agents can alert the user; this injection must be throttled to a few times per hour to avoid pestering the user.

## Consequences

- Distillation confirmed working immediately after config fix — next distill call succeeded and published a real title.
- Prior debounce/timing theory is now historical; root cause was a dead LLM endpoint plus silent error swallowing.
- New product behavior mandated: persistent distillation failures must surface to agents via throttled context injection, not remain invisible.

## Open Tail

- The throttled agent-facing notice for persistent distillation failure has been directed but not yet implemented — investigation into existing error-surfacing infrastructure was launched at end of session.
- The fabric_context path's `pubkey_ref` whitelisted-host misattribution remains unfixed.

## Evidence

- transcript lines 1117-1118
- transcript lines 1120-1232
- transcript lines 1262-1262
- transcript lines 1283-1317
- transcript lines 1384-1401

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-4-distillation-silent-failure-root-cause-and.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-4-distillation-silent-failure-root-cause-and.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-4-distillation-silent-failure-root-cause-and.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-4-distillation-silent-failure-root-cause-and.json)

---
type: episode-card
date: 2026-06-29
session: 38650a40-2fcc-452f-9b6a-9250a9c76c95
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/38650a40-2fcc-452f-9b6a-9250a9c76c95.jsonl
salience: root-cause
status: active
subjects:
  - daemon-inhibit
  - hook-observation
  - session-start
supersedes: []
related_claims: []
source_lines:
  - 275-316
captured_at: 2026-06-29T11:15:48Z
---

# Episode: Daemon-inhibit fail-open path produces spurious 'no session_id' errors when agents launched without tenex-edge

## Prior State

It was assumed that launching an agent directly (without `tenex-edge launch`) would cause the first hook to organize the session — i.e., auto-spawn the daemon and mint a session id. The inhibit mechanism (`tenex-edge stop`) was understood as a clean shutdown control, but its interaction with the observation layer's error handling was not traced.

## Trigger

User observed repeated 'no session id' errors across multiple agent sessions in a screenshot and hypothesized they occur when launching claude/opencode/grok directly instead of via `tenex-edge launch`. User explicitly requested diagnosis, not a fix.

## Decision

No code change was made (user said 'dont fix anything'), but the root cause was definitively traced: when the daemon is inhibited, `daemon_call_hook_async_with_items` returns `Ok(Null)` as a fail-open sentinel; `observation.rs:62` then does `Null["session_id"].as_str()` which yields `None`, triggering the `.context("daemon returned no session_id")` error. The inhibit check wins over session organization — the first hook bails immediately rather than auto-spawning the daemon. Two distinct failure modes were distinguished: (1) `missing-session-id` from a truncated hook payload (guard at hooks.rs:259 returns early), and (2) `daemon returned no session_id` from the inhibit/null cascade.

## Consequences

- The fail-open `Ok(Null)` contract in daemon_call_hook_async_with_items is not consumed safely by observation.rs — it treats Null as an error rather than a deliberate 'no session' state.
- Any agent launched directly while the inhibit file exists will produce these errors on every session-start hook until the inhibit is cleared.
- A fix would require either: clearing/bypassing inhibit when an agent launches outside `tenex-edge launch`, or making observation.rs handle the Null return from the inhibited path gracefully.

## Open Tail

- No fix was applied — user explicitly deferred. The decision of whether to bypass inhibit on direct agent launch vs. making observation.rs null-tolerant remains unresolved.

## Evidence

- transcript lines 275-316

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-06-29-1-daemon-inhibit-fail-open-path-produces.json`](transcripts/2026-06-29-1-daemon-inhibit-fail-open-path-produces.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-06-29-1-daemon-inhibit-fail-open-path-produces.json`](transcripts/raw/2026-06-29-1-daemon-inhibit-fail-open-path-produces.json)

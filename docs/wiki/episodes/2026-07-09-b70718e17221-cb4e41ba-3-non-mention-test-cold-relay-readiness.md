---
type: episode-card
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: root-cause
status: active
subjects:
  - non-mention-test-race
  - relay-readiness-wait
  - test-synchronization-pattern
supersedes: []
related_claims: []
source_lines:
  - 5828-5840
  - 6000-6014
captured_at: 2026-07-09T22:26:36Z
---

# Episode: non_mention test cold-relay readiness race: wait for channel materialization before send

## Prior State

De-flake agent concluded 'no remaining test-sync races' after the semaphore fix, claiming the suite was deterministically green. The non_mention test sent a channel message immediately after two session_start calls without waiting for the sender's channel to materialize on the relay.

## Trigger

Verification run without the readiness wait showed 3/5 failures with elevated runtimes (43–59s) even with the lock-free relay and semaphore fix in place — proving a distinct, additional race in the non_mention test specifically.

## Decision

Keep the readiness wait: before `channel send`, wait for the sender's channel to materialize on the relay via `wait_until`. Extracted the non_mention test into its own submodule file (`messaging/non_mention.rs`) to satisfy the LOC ratchet (messaging.rs 361 < 453 baseline). Established the synchronization doctrine: wait for relay-backed state to materialize before asserting or publishing, not bump timeouts.

## Consequences

- non_mention test is now deterministic (passes in ~28s, no 90s stall)
- Establishes reusable pattern: use wait_until for relay propagation, never bump timeouts
- Test extracted to submodule following existing repo pattern (inbox_rows, target_wire)
- Residual per-test propagation races in other tests (pty_bootstrap, freeze) confirmed as a separate, additional issue

## Open Tail

- Other tests (pty_bootstrap, freeze_39000_39002) still intermittently fail 1/10 — continuation agent launched to hunt and fix all remaining propagation races with 15+ consecutive green bar

## Evidence

- transcript lines 5828-5840
- transcript lines 6000-6014

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-b70718e17221-cb4e41ba-3-non-mention-test-cold-relay-readiness.json`](transcripts/2026-07-09-b70718e17221-cb4e41ba-3-non-mention-test-cold-relay-readiness.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-3-non-mention-test-cold-relay-readiness.json`](transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-3-non-mention-test-cold-relay-readiness.json)

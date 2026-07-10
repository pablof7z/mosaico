---
type: episode-card
date: 2026-07-10
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: root-cause
status: active
subjects:
  - integration-test-flakiness
  - croissant-semaphore-leak
  - posix-named-semaphore-exhaustion
supersedes:
  - 2026-07-09-b70718e17221-cb4e41ba-2-integration-test-flakiness-root-cause-croissant
related_claims: []
source_lines:
  - 5682-5735
  - 5826-5840
  - 6034-6043
captured_at: 2026-07-10T00:10:32Z
---

# Episode: Integration test flakiness root cause: croissant relay leaks POSIX named semaphores on SIGKILL

## Prior State

Integration suite flakiness (~1-in-3 red, ~40 of 46 tests failing together) was believed to be test-synchronization races or 'preexisting noise.' A prior `non_mention` defensive wait was added treating a symptom (degraded/slow relay) rather than the cause.

## Trigger

De-flake agent ran 14 baseline full-suite runs showing not independent flakes but a progressive collapse (run 1 = 46/46, run 14 = 43 fail). All failures were `mdb_env_open: no space left on device` with 50+ GiB disk free. A minimal C probe (`sem_open` in a loop) returned errno 28 (ENOSPC) after only 2 opens — the macOS POSIX named-semaphore table (`kern.posix.sem.max` = 10000) was saturated.

## Decision

Root cause identified as infrastructure bug: croissant (NIP-29 test relay) is SIGKILL'd by `TestRelay::Drop`, so its LMDB never cleanly closes and leaks 2 named semaphores per relay per run. Filed as GitHub issue #329 for the durable external fix. In-repo: `TestRelay` now reclaims its NIP-29 data dir on drop (fixing a secondary disk leak of 2182 orphaned trees). A lock-free croissant build (`MDB_NOLOCK`) placed at `/tmp/croissant-smallmap/croissant` as immediate unblock. 7 genuine per-test propagation races also fixed with `wait_until` predicates — zero timeout bumps, sleeps, or `#[ignore]`.

## Consequences

- Issue #329 tracks durable croissant fix (SIGTERM handler / `MDB_NOLOCK` / `sem_unlink` after `sem_open`)
- Memory file corrected with the real root cause, replacing prior 'test-sync race' belief
- Integration suite now deterministic: 40+ consecutive green runs (agent's 20 + independent 15 + final 5)
- Lock-free relay at `/tmp` is ephemeral — must be rebuilt after reboot or pointed via `$NIP29_RELAY_BIN`
- `non_mention` defensive wait confirmed necessary (3/5 red without it even with lock-free relay) and kept
- PRs #328 (gap coverage) and #330 (de-flake + production fix) merged to master

## Open Tail

- Croissant external fix (#329) not yet implemented — depends on external repo
- Lock-free relay binary in `/tmp` will not survive reboot
- Nothing deployed to live fleet — production rollout is user's explicit call

## Evidence

- transcript lines 5682-5735
- transcript lines 5826-5840
- transcript lines 6034-6043

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-10-b70718e17221-cb4e41ba-3-integration-test-flakiness-root-cause-croissant.json`](transcripts/2026-07-10-b70718e17221-cb4e41ba-3-integration-test-flakiness-root-cause-croissant.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-10-b70718e17221-cb4e41ba-3-integration-test-flakiness-root-cause-croissant.json`](transcripts/raw/2026-07-10-b70718e17221-cb4e41ba-3-integration-test-flakiness-root-cause-croissant.json)

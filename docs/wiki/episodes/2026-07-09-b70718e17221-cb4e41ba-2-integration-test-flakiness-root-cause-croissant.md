---
type: episode-card
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: root-cause
status: superseded
subjects:
  - test-relay-lifecycle
  - croissant-semaphore-leak
  - posix-semaphore-exhaustion
supersedes: []
related_claims: []
source_lines:
  - 5682-5735
  - 6034-6043
captured_at: 2026-07-09T22:26:36Z
---

# Episode: Integration test flakiness root cause: croissant LMDB POSIX semaphore leak on SIGKILL

## Prior State

daemon_integration suite was known to be 'timing-flaky' and attributed to test-synchronization races or 'preexisting noise' to be excused. TestRelay had a Drop that SIGKILL'd the croissant relay child. Thousands of test runs had accumulated orphaned nip29-relay-test-* data directories with 100GB LMDB map files.

## Trigger

De-flake agent ran 14 consecutive full-suite runs and observed progressive collapse (46→2→16→43 failures) — not independent flakes but a system-wide resource exhaustion pattern. Failures were all `mdb_env_open: ENOSPC` despite 50+ GiB free disk. A minimal C probe confirmed POSIX named-semaphore table was saturated (errno 28 after 2 opens).

## Decision

Root cause identified: croissant's LMDB opens 2 POSIX named semaphores per env (`/MDBr<hash>` `/MDBw<hash>`), only `sem_unlink`s them on clean `mdb_env_close`, but `TestRelay::Drop` sends SIGKILL so cleanup never runs — 4 semaphores leaked per relay per run, exhausting macOS `kern.posix.sem.max=10000`. Filed issue #329 for durable croissant fix. Applied TestRelay data-dir reclaim in-repo. Built a lock-free croissant relay (MDB_NOLOCK patch) at `/tmp/croissant-smallmap/croissant` as unblock vehicle.

## Consequences

- Suite runs deterministically green (12/12) once the relay can start — proving there are no inherent test-logic races beyond the semaphore exhaustion
- Croissant needs a durable fix: SIGTERM handler calling mmmm.Close(), or sem_unlink after sem_open, or MDB_NOLOCK (issue #329)
- macOS semaphore table must be cleared via reboot or `sudo sysctl -w kern.posix.sem.max=100000` to unblock current state
- Lock-free relay at /tmp is ephemeral — must be rebuilt after reboot
- TestRelay now reclaims its NIP-29 data dir on drop and on startup-failure paths, stopping the secondary disk leak

## Open Tail

- Croissant signal handler / MDB_NOLOCK fix not yet implemented (tracked in issue #329)
- Lock-free relay binary in /tmp will not survive reboot

## Evidence

- transcript lines 5682-5735
- transcript lines 6034-6043

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-b70718e17221-cb4e41ba-2-integration-test-flakiness-root-cause-croissant.json`](transcripts/2026-07-09-b70718e17221-cb4e41ba-2-integration-test-flakiness-root-cause-croissant.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-2-integration-test-flakiness-root-cause-croissant.json`](transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-2-integration-test-flakiness-root-cause-croissant.json)

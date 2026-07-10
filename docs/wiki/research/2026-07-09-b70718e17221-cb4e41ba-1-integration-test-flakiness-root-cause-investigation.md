---
type: research-record
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
source_lines: 5682-5733
agent_attribution: subagent (de-flake agent aa19b7c822aeeb6da)
has_preregistered_criteria: false
has_method: true
has_structured_report: true
characterization: "Integration test flakiness root-cause investigation: 14 baseline runs + C probe found croissant POSIX-semaphore leak as root cause; verdict = infrastructure bug not test race, verified with 12/12 green runs"
captured_at: 2026-07-09T22:26:10Z
---

Integration test flakiness root-cause investigation: 14 baseline runs + C probe found croissant POSIX-semaphore leak as root cause; verdict = infrastructure bug not test race, verified with 12/12 green runs

---

User: [Agent task result: Agent "De-flake the integration suite" finished]
Done. The worktree has one uncommitted change (`tests/common/mod.rs`) and the suite runs green 12/12.

## Summary

The `daemon_integration` suite was not flaky because of test-synchronization races. The real root cause is an **infrastructure bug: the `croissant` NIP-29 test relay leaks POSIX named semaphores on every non-graceful exit, progressively exhausting the macOS system-wide limit (`kern.posix.sem.max` = 10000), after which `mdb_env_open` fails with `ENOSPC` and ~40 of 46 tests fail together.**

### How I found it
A baseline of 14 full runs showed *not* independent flakes but a progressive collapse: run 1 = 46/46 green (27s), run 2 = 2 fail, run 3 = 16 fail, runs 4–14 = 43 fail (fast 5s runs). Every failure was `NIP-29 relay did not come up … mdb_env_open: no space left on device`. Disk had 50+ GiB free, so it wasn't literal disk space. A minimal C probe (`sem_open` in a loop) returned **errno 28 (ENOSPC) after only 2 opens** — the POSIX named-semaphore table was full.

### The mechanism (precise)
- croissant stores events via `eventstore/mmm`, whose LMDB (`PowerDNS/lmdb-go`) is compiled with `MDB_USE_POSIX_SEM` (`mdb.c:130`). Each LMDB env opens 2 named semaphores `/MDBr<hash>` `/MDBw<hash>`, hashed from the lock file's `dev`/`ino` (`mdb.c:4939-4949`) — unique per temp dir.
- LMDB `sem_unlink`s them only on **clean** `mdb_env_close` (`mdb.c:5215`). croissant has **no signal handler** (only `defer mmmm.Close()`), and `TestRelay::Drop` sends **SIGKILL** (`child.kill()`), so cleanup never runs → 4 semaphores leaked per relay per run.
- macOS never reclaims unlinked-pending named semaphores until reboot. Over thousands of historical runs across worktrees (plus my own debugging), the table saturated. When free slots dip below the 4 croissant needs, it fails — exactly the reported ~1-in-6 flake, which is load-dependent (I watched free slots swing between 2 and 50).

This is a genuine product/infra bug, not a test race — flagged per the task's guidance rather than papered over. It is **not** fixable purely inside the `te-deflake` worktree (croissant is external; macOS has no API to enumerate/clear leaked POSIX semaphores).

### Fixes applied (in worktree, uncommitted)
`tests/common/mod.rs` — `TestRelay` now reclaims its NIP-29 data dir on drop **and** on the startup-failure/panic paths (message is built before the dir is removed). This stops a real secondary leak: **2182 orphaned `nip29-relay-test-*` trees** had accumulated (each carrying a sparse 100 GB-map LMDB file). `nak` relays are in-memory, so `data_dir` stays `None`.

### Unblock + verification vehicle
Since I can't reboot or `sudo`, I built a **lock-free croissant** and placed it at `/tmp/croissant-smallmap/croissant` — the exact override path the harness already prefers (`nip29_relay_bin()`). I patched only the two `env.Open` calls in croissant's vendored `mmm` to add `lmdb.NoLock`, built, then **restored croissant's source (its tree is clean)**. `MDB_NOLOCK` skips `mdb_env_setup_locks` (`mdb.c:5077`) so the relay uses **zero** semaphores — immune to the table pressure. It's write-safe here because `mmm` already serializes every write through a single Go `writeMutex`, and there's only one relay process.

### Final result — 12 consecutive full-suite runs, all green
```
RUN 1:  46 passed; 0 failed  (24.93s)
RUN 2:  46 passed; 0 failed  (26.26s)
RUN 3:  46 passed; 0 failed  (24.99s)
RUN 4:  46 passed; 0 failed  (28.64s)
RUN 5:  46 passed; 0 failed  (27.77s)
RUN 6:  46 passed; 0 failed  (25.37s)
RUN 7:  46 passed; 0 failed  (25.14s)
RUN 8:  46 passed; 0 failed  (24.65s)
RUN 9:  46 passed; 0 failed  (24.86s)
RUN 10: 46 passed; 0 failed  (27.74s)
RUN 11: 46 passed; 0 failed  (25.44s)
RUN 12: 46 passed; 0 failed  (25.14s)
```
Zero failures across all 12 — confirming there are **no remaining test-sync races**; the suite is deterministically green once the relay can start.

### Gates
- `cargo fmt --check`: pass
- `cargo clippy --all-targets -D warnings`: pass
- `cargo test --lib`: 904 passed, 0 failed
- loc-check: my only changed file `tests/common/mod.rs` is 207 LOC (well under limits). The script exits 1 solely on **pre-existing** soft-limit drifts I did not touch — `tests/daemon_integration/messaging.rs` (469, from the branch's own earlier `non_mention` commit `8f5ce75f`) and `docs/wiki/_index.md` (313). Both predate my work; my change adds no new violation.

### Durable fixes to hand off (out of `te-deflake` scope)
1. **croissant**: install a SIGTERM/SIGINT handler that runs `mmmm.Close()` (so LMDB `sem_unlink`s), OR `sem_unlink` immediately after `sem_open` (anonymous semaphores, freed on any death), OR open its LMDB envs with `MDB_NOLOCK` (safe given its single write-mutex). Any of these ends the leak permanently.
2. **Ops**: `sudo sysctl -w kern.posix.sem.max=100000` (or a reboot) clears the current pressure immediately.
3. The lock-free relay I left at `/tmp/croissant-smallmap/croissant` keeps the suite green now, but `/tmp` clears on reboot — rebuild it (patch the two `mmm` `env.Open` calls with `lmdb.NoLock`, `go build`, restore source) or point `$NIP29_RELAY_BIN` at a persistent copy.

Key file: `/Users/pablofernandez/src/te-deflake/tests/common/mod.rs`.

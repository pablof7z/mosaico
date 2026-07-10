---
type: research-record
date: 2026-07-10
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
source_lines: 6056-6118
agent_attribution: a067926378f0c4ada (residual de-flake agent)
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Residual per-test flake investigation: found and fixed 6 propagation races + 1 production bug across ~85 runs; verdict = 20/20 consecutive green, all flakes eliminated."
captured_at: 2026-07-10T00:10:13Z
---

Residual per-test flake investigation: found and fixed 6 propagation races + 1 production bug across ~85 runs; verdict = 20/20 consecutive green, all flakes eliminated.

---

User: [Agent task result: Agent "Fix residual per-test flakes" finished]
All four gates pass (exit 0). The changes are left uncommitted as required. Here is my final report.

## Summary

Eliminated the residual per-test flakiness in the `daemon_integration` suite (worktree `/Users/pablofernandez/src/te-deflake2`, branch `fix/deflake-residual`). Final clean confirmation: **20 consecutive full-suite runs, every one `46 passed; 0 failed`** (exceeds the 15+ requirement). All gates green: `fmt-check=0`, `loc-check=0`, `lint=0`, `test-unit=0` (911 unit tests pass). Changes left uncommitted.

### Flaky tests found (across ~85 full runs) and the fix for each

1. **`channels::pty_bootstrap::pty_spawn_bootstraps_session_without_child_session_start_hook`** — ~2/20. Panic at `pty_bootstrap.rs:93`: asserted `is_channel_member` after a single `refresh_channel_members` call, racing the relay-materialized 39002 membership snapshot. **Fix:** wrapped the assertion in `wait_until(25s, || { refresh_channel_members; is_channel_member })`.

2. **`channels::orchestration::orchestration_session_uses_existing_group_without_minting`** — ~1/20. Panic at `orchestration.rs:41`: the channel row materializes (from the 39002 members snapshot) with an **empty `name`** before its 39000 metadata arrives; the wait only checked `get_channel().is_some()`. **Fix:** strengthened the wait predicate to require `name == "issue-42" && parent == "tmp"`.

3. **`channels::session_rooms::opencode_style_session_without_id_mints_room_via_pid`** — fixed proactively (same "row before parent-metadata" class): unguarded `!is_root_channel` at line 329 could see the room transiently look root. **Fix:** poll until the row exists and is non-root. (Extracted this + a sibling into `channels/session_rooms/root_selection.rs` to keep `session_rooms.rs` under its LOC baseline.)

4. **`channels::session_lifecycle::generated_management_key_self_grants_on_existing_user_owned_channel`** — ~1/25. Panic at line 151: a redundant **fresh re-read** *after* the `wait_until` already validated the self-grant — a TOCTOU against the relay's converging (flapping) member set. **Fix:** removed the redundant post-wait re-read in this test and its sibling `session_start_without_tenex_private_key_...` (the `wait_until` is the assertion).

5. **`freeze::freeze_39000_39002_idempotency_no_member_duplication`** — panic at `freeze.rs:121`. The idempotency check seeded `replace_channel_members` on the **live room's real id** with `ts=9_000_000`; `replace_role` is high-water-mark guarded (`src/state/members.rs:78`), so the daemon's real relay 39002 (ts ~1.7e9) wins and clobbers the seed. **Fix:** run the idempotency check on a **dedicated fake channel id** the relay never authors 39002 for (`freeze-39002-members`) — mirroring how the sibling 39000 metadata check already uses `freeze-39000-meta`.

6. **`messaging::chat_write_stdin_enqueues_live_channel_chat_for_receiver`** — panic at `messaging.rs:213`. A single `channel read` raced the sent message's materialization into the readable store. **Fix:** poll the read inside `wait_until` until the body renders.

### One product bug fixed (daemon startup)

7. **`messaging::chat_commands_require_channel_when_session_joined_to_multiple_channels`** — ~1/46. Panic: `connect: daemon did not answer handshakes within 30s`. Root cause in `src/daemon/server/lifecycle.rs`: the accept loop (line ~119) is spawned only **after** `Transport::connect_with_indexer(...).await`, so relay-connect latency stalls even store-only RPC handshakes — contradicting the code's own comment that store-only RPCs "serve instantly even when the relay is slow." **Fix (`src/transport.rs`):** initiate `client.connect()` in a background `tokio::spawn` instead of awaiting it on the startup critical path; `warmup()` still awaits real connectivity + AUTH off the critical path. This is the only production-code change.

### Not changed (honest notes)
- **`process::hooks::hooks_fail_open_without_spawning_daemon`** failed twice, but **only** while I had several concurrent background monitor loops running — it is a wall-clock budget test (19 subprocess spawns must total <10s), and it passed in all ~60 runs in an unloaded environment (including the final clean 20). It is not a relay race and not caused by my changes; it is load-sensitive. I deliberately ran the final confirmation with no concurrent monitors and it passed 20/20.

### Files changed (all absolute)
- `/Users/pablofernandez/src/te-deflake2/src/transport.rs` (production: background connect)
- `/Users/pablofernandez/src/te-deflake2/tests/daemon_integration/channels/pty_bootstrap.rs`
- `/Users/pablofernandez/src/te-deflake2/tests/daemon_integration/channels/orchestration.rs`
- `/Users/pablofernandez/src/te-deflake2/tests/daemon_integration/channels/session_lifecycle.rs`
- `/Users/pablofernandez/src/te-deflake2/tests/daemon_integration/channels/session_rooms.rs`
- `/Users/pablofernandez/src/te-deflake2/tests/daemon_integration/channels/session_rooms/root_selection.rs` (new, LOC-split)
- `/Users/pablofernandez/src/te-deflake2/tests/daemon_integration/freeze.rs`
- `/Users/pablofernandez/src/te-deflake2/tests/daemon_integration/messaging.rs`

### Final 20 consecutive runs (all `46 passed; 0 failed`)
```
run 1: ...46 passed; 0 failed... finished in 34.91s
run 2: ...46 passed; 0 failed... finished in 29.18s
run 3: ...46 passed; 0 failed... finished in 30.09s
run 4: ...46 passed; 0 failed... finished in 28.70s
run 5: ...46 passed; 0 failed... finished in 28.58s
run 6: ...46 passed; 0 failed... finished in 29.92s
run 7: ...46 passed; 0 failed... finished in 29.08s
run 8: ...46 passed; 0 failed... finished in 32.36s
run 9: ...46 passed; 0 failed... finished in 29.02s
run 10: ...46 passed; 0 failed... finished in 29.07s
run 11: ...46 passed; 0 failed... finished in 31.36s
run 12: ...46 passed; 0 failed... finished in 29.33s
run 13: ...46 passed; 0 failed... finished in 28.97s
run 14: ...46 passed; 0 failed... finished in 29.98s
run 15: ...46 passed; 0 failed... finished in 28.74s
run 16: ...46 passed; 0 failed... finished in 31.20s
run 17: ...46 passed; 0 failed... finished in 28.92s
run 18: ...46 passed; 0 failed... finished in 29.36s
run 19: ...46 passed; 0 failed... finished in 28.53s
run 20: ...46 passed; 0 failed... finished in 29.35s
```

Residual risk: the two known non-propagation flakes I addressed (chat_commands daemon-startup handshake, hooks_fail_open wall-clock budget) did not recur in the final 20 clean runs, but both were rare (~2%) and load/timing sensitive; the transport fix addresses the former by design and the latter only manifested under my own concurrent monitoring load. No `#[ignore]`, blanket sleeps, or timeout bumps were used.

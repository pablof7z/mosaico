# Review: session-state rearchitecture at a41bd570

Line numbers below refer to the `a41bd570` tree, not the current dirty worktree.

## Findings

### Critical: Claude/Codex hooks still mutate by harness id, so canonical transitions no-op

The daemon explicitly treats `session_start.session_id` / `harness_session_id` as an alias, not identity (`src/daemon/server.rs:493`, `src/daemon/server.rs:499`, `src/daemon/server.rs:576`). The hook receives the daemon-minted canonical id in `canonical`, but only echoes it for `opencode` (`src/cli/hooks.rs:228`, `src/cli/hooks.rs:238`). For non-echo hosts (`claude-code`, `codex`) the later `user-prompt-submit` reasserts the observation and then calls `turn_start(sid.clone(), ...)` with the raw harness id (`src/cli/hooks.rs:275`, `src/cli/hooks.rs:288`). `turn_start` sends that raw value as `"session"` (`src/cli/turn.rs:27`), and the daemon calls `get_turn_state`, `start_turn`, transcript setters, and later `end_turn` with `p.session` directly (`src/daemon/server.rs:1475`, `src/daemon/server.rs:1479`, `src/daemon/server.rs:1572`, `src/daemon/server.rs:1578`).

Those transition methods update `session_state WHERE session_id=?`, so a harness id alias does not touch the canonical row. The later `get_session(&p.session)` can resolve the alias for context (`src/daemon/server.rs:1495`), which masks the failure while busy/title/distill/status_outbox never update. `session_end` has the same bug: it resolves `rec`, then cancels and ends `p.session` instead of `rec.session_id` (`src/daemon/server.rs:797`, `src/daemon/server.rs:800`, `src/daemon/server.rs:805`), leaving the canonical session alive.

### Critical: turn transitions have two owners and can double-increment `turn_id`

For canonical ids, the daemon RPC and runtime both call the canonical transition methods. `rpc_turn_start` calls `start_turn` immediately (`src/daemon/server.rs:1475`, `src/daemon/server.rs:1479`), and the runtime later observes `turn_state.working` and calls `start_turn` again on the same turn edge (`src/runtime.rs:210`, `src/runtime.rs:219`). The same duplication exists at turn end: `rpc_turn_end` calls `end_turn` (`src/daemon/server.rs:1572`, `src/daemon/server.rs:1578`), then the runtime falling-edge path calls `end_turn` again (`src/runtime.rs:289`, `src/runtime.rs:292`).

That violates the single-owner aggregate design. Once the alias bug above is fixed, each turn can bump `turn_id` twice, advance `state_version` twice, enqueue two status publishes, and move the distillation base from the hook transition to the runtime transition. The code needs one owner for start/end transitions; the other path should only set/read a command cursor.

### High: NIP-40 expiration is not re-armed by heartbeat

The codec/provider path can publish an expiration tag (`src/codec/kind1.rs:203`, `src/fabric/provider.rs:465`), and the outbox drainer builds a `Status` with `expires_at = now + STATUS_TTL_SECS` (`src/daemon/server.rs:2950`, `src/daemon/server.rs:2961`). But runtime heartbeats only call `heartbeat_session` and `touch_session` (`src/runtime.rs:147`, `src/runtime.rs:168`), while `heartbeat_session` only updates `last_seen` and explicitly does not enqueue (`src/state.rs:1591`, `src/state.rs:1596`). The only publisher loop drains `status_outbox` (`src/daemon/server.rs:2975`) and there is no heartbeat publisher path.

Impact: a live but idle session publishes one kind:30315 after registration/start/end changes, then the relay event expires about 90 seconds later despite local heartbeats every 30 seconds. Remote liveness becomes "gone" for a live session, so the core NIP-40 design is not achieved.

### High: upgrade/restart does not preserve existing sessions

The migration creates empty `session_state` / `peer_session_state` tables and drops the legacy status tables (`src/state.rs:296`, `src/state.rs:340`, `src/state.rs:578`, `src/state.rs:582`). I do not see a migration from existing `sessions` rows into `session_state`. Startup reconciliation then rebuilds only from `live_session_snapshots`, which reads `session_state` (`src/daemon/server.rs:3157`, `src/daemon/server.rs:3159`; `src/state.rs:1681`, `src/state.rs:1687`). `who` also reads local rows from `live_session_snapshots` (`src/cli/who.rs:154`, `src/cli/who.rs:157`).

On an existing populated `state.db`, pre-refactor live sessions remain in `sessions` but are invisible to the new `who` path and are not revived by `reconcile_sessions`. The next hook may create a new canonical row, but that is not session survival; it is a late re-registration with old rows left behind.

### High: turn-start deltas are not self-excluded

`push_turn_fabric_block` has no current-session parameter. On first turn it renders `load_who_snapshot(...)` (`src/cli/who.rs:304`, `src/cli/who.rs:305`), and that function includes all local `session_state` rows for the project (`src/cli/who.rs:157`, `src/cli/who.rs:189`, `src/cli/who.rs:196`). On later turns it calls `build_status_delta(..., None)` (`src/cli/who.rs:315`, `src/cli/who.rs:316`), so the current session is not excluded from `status_delta_since`.

Because `rpc_turn_start` itself calls `start_turn` before assembling context (`src/daemon/server.rs:1475`, `src/daemon/server.rs:1479`), the current session's just-started busy transition is eligible to show up as a "changed" delta in its own prompt. `turn_check` does pass `Some(&rec.session_id)` (`src/cli/turn.rs:188`, `src/cli/turn.rs:191`); turn-start should do the same.

### Medium: `turn_check` is not a pure read

`rpc_turn_check` calls `turn_check_due` (`src/daemon/server.rs:1544`, `src/daemon/server.rs:1549`), and `turn_check_due` advances `turn_state.last_check_at` (`src/state.rs:1201`, `src/state.rs:1236`). That is an intentional write on the PostToolUse path, despite the desired "pure read" semantics. It can also move the delta cursor before the context is actually delivered or consumed.

### Medium: pending outbox rows publish the current snapshot, not their version

`status_outbox` is keyed by `(session_id, state_version)` (`src/state.rs:364`, `src/state.rs:372`), but `pending_status_outbox` joins pending rows to `session_state` only by `session_id` (`src/state.rs:1902`, `src/state.rs:1904`) and returns the current snapshot (`src/state.rs:1916`). The drainer then publishes one status per pending row and marks each row's version published (`src/daemon/server.rs:2975`, `src/daemon/server.rs:2998`, `src/daemon/server.rs:3002`).

If versions 1, 2, and 3 are pending, all three publishes can carry the version-3 snapshot. That creates duplicate current status events and means the outbox row is not really "the desired publish for this version." Either coalesce pending rows to the latest version before publishing, or store/validate the snapshot version being published.

### Medium: alias lookup for explicit session ids ignores alias namespace

The schema namespaces aliases by `(harness, external_id_kind, external_id)` (`src/state.rs:326`, `src/state.rs:332`), but `get_session` falls back with `WHERE external_id=?1 ORDER BY created_at DESC` (`src/state.rs:636`, `src/state.rs:646`). That collapses harness id, resume id, tmux pane, and watch pid into one raw string namespace. A collision can route a hook or CLI command to the newest unrelated canonical session.

Alias-aware lookups need either the harness/kind context or a collision-safe rule that refuses ambiguous raw ids instead of picking newest.

### Medium: transition methods are not SQLite transactions

The comments promise one transaction (`src/state.rs:1303`, `src/state.rs:1305`), but `register_or_reassert_session` runs alias lookup, supersede/insert/reassert, outbox enqueue, and alias writes as separate autocommit statements (`src/state.rs:1315`, `src/state.rs:1324`, `src/state.rs:1331`, `src/state.rs:1335`). `start_turn` likewise updates `session_state`, then updates `turn_state`, then enqueues the outbox as separate statements (`src/state.rs:1517`, `src/state.rs:1530`, `src/state.rs:1536`).

Within the daemon mutex this avoids in-process interleaving, but it is not crash-safe and does not provide the atomicity the design claims. A failure after the aggregate update but before enqueue leaves a state change with no status publish; a failure during supersede/mint can retire the old session without a complete new alias set.

## Confirmed pieces

- The wire codec does add NIP-40 `expiration` when `expires_at` is set (`src/codec/kind1.rs:203`).
- `derive_status` is a real shared projection and suppresses activity when not busy (`src/session.rs:280`, `src/session.rs:293`).
- The runtime no longer constructs `DomainEvent::Status`; status publishing is routed through provider `set_status` from the drainer (`src/runtime.rs:125`, `src/daemon/server.rs:2998`, `src/fabric/provider.rs:465`).
- Live Rust references to the deleted `agent_status` / `session_status` tables are gone except for the DROP migration and stale comments/docs. The remaining active materializer writes peer status into `peer_session_state` (`src/fabric/kind1/materializer.rs:45`, `src/state.rs:1787`).

## Bottom line

The commit has the right table names and several good projections, but it does not yet deliver the intended architecture. The largest blockers are identity normalization at RPC boundaries, duplicate transition ownership, and missing heartbeat republish. Until those are fixed, the canonical `session_state` row is not reliably the live source of truth for ordinary Claude/Codex sessions, and remote liveness will expire for live idle sessions.

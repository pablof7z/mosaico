# Daemon-owned NIP-29 groups: auto-create + auto-add members

## Context

Today tenex-edge uses NIP-29 **passively**: every fabric event (presence kind:30315,
activity/mention kind:1) carries an `["h", <project-slug>]` tag, and `nip29.f7z.io`
accepts those events for any client-chosen group id without the group being formally
created. Group lifecycle is entirely relay-owned; the daemon only *reads* kind:39000
metadata to show project descriptions (`server.rs` `handle_incoming`, the
`if event.kind.as_u16() == 39000` branch → `upsert_project_meta`).

We now want the singleton daemon to actively **own** the NIP-29 groups for the
operator's projects:

1. Keep a persistent subscription open so the daemon always knows which groups it owns
   (their metadata/admins/members).
2. When a session starts in a project whose group doesn't exist yet, **auto-create**
   the group (kind:9007), signed by the operator's `userNsec`.
3. When a session starts for an agent that isn't yet a member, **auto-add** that agent
   (kind:9000 put-user), signed by `userNsec`, *before* the engine starts publishing —
   so the agent finds itself already a member when it publishes presence.

**Decided product behavior (user-confirmed): closed/managed groups.** Formalizing a
group flips `nip29.f7z.io` into membership-enforced mode, so only pubkeys we explicitly
add can post. This is intended. **Consequence to document:** cross-operator peer agents
that today post into a shared-slug project group by using its `h` tag will be rejected
once we own that group. The same project slug maps to a single relay group id, so the
first operator to formalize a slug becomes its gatekeeper. This is an accepted tradeoff.

`userNsec` is the operator/admin key (`Config::user_nsec`, an `Option<String>`); the
group creator becomes admin and signs all membership changes. Agents keep their own
per-machine keys for presence/activity/mentions — those are unchanged.

## Risk gate — probe the live relay FIRST (Step 0, do before any wiring)

The relay's exact rules are the only real unknown and the codebase can't answer them.
Mirror the existing empirical-probe precedent (`tests/relay_probe.rs`) with a new
`tests/nip29_probe.rs` (ignored by default; run explicitly) that connects to
`nip29.f7z.io` with a throwaway key and answers, in order:

1. **Does 9007 honor a client-chosen group id via the `h` tag, or does the relay assign
   its own id?** Load-bearing: the entire model assumes `group id == project slug`. If
   the relay assigns ids, the design must change (e.g. store a slug→groupid map). Stop
   and reassess if this fails.
2. **Can you 9007 a slug that already has passive activity?** (Relay may auto-vivify
   unmanaged groups; 9007 may return "already exists". Determine whether such a group
   can still be claimed/managed, e.g. via 9002, or not.)
3. **Ordering:** does put-user (9000) require the group to exist first? What role tag
   does relay29 expect (`["p", <pk>, "member"]` vs bare `["p", <pk>]`)?
4. **Are `previous`/timeline-reference tags required** on 9007/9000? (relay29 anti-spam.)
5. **Default access mode of a freshly-created group** — is it already closed+private, or
   must we send a 9002 edit-metadata with `["closed"]`/`["private"]` to lock it? Confirm
   by posting a non-member event after create+add and checking it is rejected.

The findings determine the exact tag sets used in Step 2. Keep the codec builders thin
enough to adjust once the probe answers these.

### Probe RESULTS (validated 2026-06-09 against nip29.f7z.io) — drives the steps below

- **Q1 ✓** Client-chosen id honored. `9007` `["h", slug]` → group `d` == slug. Signer
  becomes admin (39001) and auto-member (39002). `group id == project slug` holds.
- **Q3 ✓** `9000` put-user `["h", slug]`, `["p", pubkey, "member"]` accepted; appears in
  39002. create→put-user ordering fine.
- **Q4 ✓** No `previous`/timeline-ref tags required.
- **Q5 ⚠️** A fresh group is **OPEN by default** — members AND non-members can post.
  Creating a group does NOT enforce membership.
- **Q5b ✓** A `9002` edit-metadata with `["closed"]` enforces it: non-member writes are
  rejected `"blocked: unknown member"`; members still write.
- **Q6 ✓** Use `["public"]`, **not** `["private"]`. `closed`+`public` enforces membership
  for WRITES but leaves READS open — required because the daemon's single connection
  authenticates as the non-member `tenex-edge-daemon` key and must still receive its
  agents' events. Verified a non-member connection can read a closed+public group.

**Recipe (signed by `userNsec`):** `9007` create `["h",slug]` → `9002`
`["h",slug],["name",slug],["closed"],["public"]` → `9000` per agent
`["h",slug],["p",pubkey,"member"]`. The 9002 lock is a one-time step at group creation;
put-user runs per new agent. (See memory `nip29-relay29-f7z-behavior`.)

## Implementation

### Step 1 — State: track owned groups, admins, members

`src/state.rs`. Reuse the existing single-writer `Store` pattern and the
`upsert_project_meta` shape. Add two tables + accessors (all writes go through the
existing `state.with_store(...)` path — never a second SQLite writer; see the
multi-writer corruption note in memory):

- `group_members(project TEXT, pubkey TEXT, role TEXT, updated_at INTEGER, PRIMARY KEY(project, pubkey))`
  with `upsert_group_member(project, pubkey, role, ts)`, `is_group_member(project, pubkey) -> bool`,
  and `replace_group_members(project, &[(pubkey, role)])` (for applying a relay 39002 snapshot).
- `owned_groups(project TEXT PRIMARY KEY, created_at INTEGER)` with
  `mark_group_owned(project, ts)` and `is_group_owned(project) -> bool`. "Owned" = we have
  published (or observed) a managed group for this slug and intend to keep it subscribed.

(39001 admins: optional. We only need to know our own `userNsec` pubkey is admin, which
is true by construction after we create. Skip a separate admins table unless the probe
shows we need to detect externally-managed groups.)

### Step 2 — Codec: NIP-29 group-management builders + subscription filters

`src/codec/kind1.rs`. These are operator-key-signed management events, distinct from the
domain `DomainEvent` flow, so expose them as **standalone builder helpers** (not new
`DomainEvent` variants) returning `EventBuilder`, mirroring how `project_tag` / `tag`
already construct tags:

- `group_create(project) -> EventBuilder` — `Kind::from(9007u16)`, tags `["h", project]`.
- `group_lock_closed(project) -> EventBuilder` — `Kind::from(9002u16)`, tags
  `["h", project]`, `["name", project]`, `["closed"]`, `["public"]`. **Required** — a fresh
  group is open until this runs (probe Q5/Q5b). `public` (not `private`) keeps reads open
  for the non-member daemon connection (Q6).
- `group_put_user(project, pubkey) -> EventBuilder` — `Kind::from(9000u16)`, tags
  `["h", project]`, `["p", pubkey, "member"]` (role tag validated by probe Q3).

Extend the **subscription filter set** so the persistent subscription also covers the
group-management addressable events for owned slugs. In `Kind1Codec::filters(&SubScope)`
(the function that currently builds the kind:1 / kind:30315 filters near the bottom of
`kind1.rs`), add a filter for kinds `39000, 39001, 39002` constrained by `#d` = the
project slug (these addressable events are **relay-authored**, so filter by `#d` group
id, never by author). This is what makes "leave the subscription open" true — it rides
the existing long-lived `client.subscribe` path; **do not** spawn a parallel loop.

### Step 3 — Daemon: create + add on session_start (signed by userNsec)

`src/daemon/server.rs`, `rpc_session_start` (currently builds the `SessionRecord`,
then calls `spawn_session`). Insert a new step **after** the session row is upserted and
**before** `spawn_session(state, ep).await?`, so membership is in place before the engine
publishes presence:

```
ensure_group_and_membership(state, &project, &id.pubkey_hex()).await;  // best-effort
```

New helper `async fn ensure_group_and_membership(state, project, agent_pubkey)`:

1. If `state.cfg.user_nsec` is `None`: log once and **return Ok** — do NOT fail
   `rpc_session_start`. (Unlike `rpc_user_prompt`, which bails: a session must still start
   for operators without a `userNsec`. It just won't get a managed group.) Reuse the
   `Keys::parse(&nsec)` pattern from `rpc_user_prompt`.
2. If `!store.is_group_owned(project)`: publish `group_create(project)` then
   `group_lock_closed(project)` (the 9002 closed+public lock — required per probe), both
   via `transport.publish_signed(builder, &user_keys)`, then `mark_group_owned(project)`.
   Tolerate an "already exists" relay error (treat as owned).
3. If `!store.is_group_member(project, agent_pubkey)`: publish
   `group_put_user(project, agent_pubkey, role)` via `publish_signed(.., &user_keys)`,
   then `upsert_group_member(project, agent_pubkey, role, now)`.
4. Call `ensure_subscription(state, project)` (already invoked by `spawn_session`, but
   calling here too guarantees the 39000/39001/39002 filters are live as soon as we own
   the group — `ensure_subscription`/`resubscribe` already de-dup projects).

Make every relay publish here best-effort (log on error, never propagate) so a flaky
relay can't block a session from starting.

### Step 4 — Daemon: cache relay group state from the open subscription

`src/daemon/server.rs`, `handle_incoming`. The 39000 branch already exists; extend it:

- 39000: keep existing `upsert_project_meta`.
- 39002 (members): parse all `p` tags → `replace_group_members(project, members)` so the
  daemon's membership cache reflects the relay's authoritative snapshot (self-heals if our
  optimistic `upsert_group_member` ever drifted). `project` here is the `d` tag value.
- 39001 (admins): optional — only wire if Step 1 kept an admins table.

This is the read side of "check which groups we own at all times": the persistent
subscription feeds the cache, and the cache makes the Step 3 create/add decisions instant
and idempotent.

### Step 5 — Seed owned groups on daemon start

`src/daemon/server.rs`, `reconcile_sessions` (runs at startup over `alive=1` sessions).
For each revived session's project, ensure its group-management filters are subscribed
(call `ensure_subscription`) and, if `is_group_owned`, that membership for the revived
agent is (re)published best-effort. This restores the open subscription for owned groups
across daemon restarts/skew-reexec without a relay-wide scan. Resist any "scan every group
on the relay to find ones I'm admin of" approach — we only track the daemon's own project
set.

## Files touched

- `tests/nip29_probe.rs` — **new**, Step 0 empirical probe (run explicitly, `#[ignore]`).
- `src/state.rs` — new `group_members` (+ optional `owned_groups`) tables and accessors.
- `src/codec/kind1.rs` — `group_create` / `group_put_user` / `group_edit_metadata`
  builders; extend `filters()` with the 39000/39001/39002 `#d`-scoped filter.
- `src/daemon/server.rs` — `ensure_group_and_membership` helper; call it in
  `rpc_session_start` before `spawn_session`; extend `handle_incoming` for 39001/39002;
  seed owned groups in `reconcile_sessions`.
- Docs: note the cross-operator lockout consequence in
  `docs/wiki/tenex-edge-messaging.md` (or identity) and update the
  `codec/kind1.rs` comment ("NIP-29 project groups are open for now…") which is no longer
  true once groups are managed.

## Reuse (don't reinvent)

- `Keys::parse(&nsec)` + `transport.publish_signed(builder, &keys)` — exact pattern in
  `rpc_user_prompt` (`server.rs`); the operator-signed-over-daemon-auth path is already
  proven (`tests/relay_probe.rs`).
- `state.with_store(...)` single-writer access for every new table write.
- `ensure_subscription` / `resubscribe` for the persistent open subscription — extend,
  don't replace.
- `project::resolve(&cwd)` already yields the slug used as the group id everywhere.
- `event_tag(event, name)` helper in `server.rs` for parsing `d`/`p` tags off 3900x events.

## Verification

1. **Probe (Step 0):** `cargo test --test nip29_probe -- --ignored --nocapture` against
   `nip29.f7z.io`. Confirm: client-chosen id honored, create→put-user ordering, role tag
   shape, timeline-ref requirement, and that a non-member event is rejected after lock.
   This gates the rest.
2. **Unit:** codec builders produce the expected kind+tags; `state.rs` member/owned
   accessors round-trip (follow existing `state.rs` test style).
3. **Integration** (extend `tests/daemon_integration.rs` / `daemon_mechanics.rs`): start a
   session in a fresh project with a `userNsec` set → assert a 9007 then 9000 are published
   and `is_group_owned` / `is_group_member` become true; start a second session for a new
   agent in the same project → assert only a 9000 (no duplicate 9007); start a session with
   `userNsec` unset → assert the session still starts (no failure) and no group events.
4. **End-to-end manual:** with a real `userNsec`, run `tenex-edge session-start` in a new
   project, then confirm on the relay that the group exists, the agent is a member, and the
   agent's subsequent presence (kind:30315) is accepted (not rejected) by the now-managed
   group. Notify via `say` when the e2e check passes.

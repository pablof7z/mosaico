# Plan: NIP-29 Subgroup Task Rooms + Cross-Device Orchestration (GH #3)

## Context

We want agents to coordinate around semi-temporary topical rooms (e.g. `tenex-edge > subgroup support`) backed by **real NIP-29 subgroups** (per nostr-protocol/nips#2319), so clients can render the hierarchy. The goal is organization, not permission isolation for its own sake.

A new CLI command `project create-group` creates a child NIP-29 group under a parent, copies the parent's human/backend admins down, and publishes **one** `kind:9` orchestration event asking the named backends to add their agents. Every backend daemon listens for these orchestration events and heeds the ones addressed to its own backend pubkey — minting the role identity, publishing its profile, adding it as a member, and spawning the harness into the child room. This one listener path serves both local and remote backends, which is what makes cross-device auto-start work from day one.

**Out of scope (handled separately):** relay subgroup-support verification (NIP-11 `nip29.subgroups`). Where the create flow must gate on it, call the other workstream's checker as a black box (`relay_supports_subgroups().await?`) and bail loudly if unsupported.

### Locked product decisions
- **Backend identity** = pubkey of `tenexPrivateKey` in `~/.tenex/config.json`, distinct from `userNsec` (the human operator). Config currently *collapses* them (`user_nsec: raw.user_nsec.or(raw.tenex_private_key)`); they must be split so the backend pubkey is first-class.
- **Any group we create** gets as admins: all `whitelistedPubkeys` + pubkey(userNsec) (if present) + pubkey(tenexPrivateKey) (the local backend).
- **A subgroup** additionally copies the **parent's live admin set** (`kind:39001`) down as admins, filtered to human/whitelisted/backend pubkeys (skip agent/session pubkeys). Adding a friend = add their pubkey as parent admin; the copy carries it into the subgroup and descendants, letting them add their own backend/agents.
- `create-group` **provisions only** (creates group, copies admins, publishes the kind:9). It does **not** spawn harnesses directly — the orchestration listener does, uniformly for local + remote.
- Receivers parse **only structured tags**; untagged prose is ignored.
- Child id: `<slugify(name)>-<random8>`; display path `parent > name`; hierarchy lives in a `parent` metadata tag, not in the id.

## Approach

### 1. Config split (`src/config.rs`)
Stop collapsing. Keep `user_nsec: Option<String>` (operator) and add `tenex_private_key: Option<String>` (backend). Add **three** distinct accessors so the three concepts that the collapsed field conflated stay independent:
- `session_ikm_nsec()` → `user_nsec.or(tenex_private_key)` — **exact** reproduction of today's collapsed value, used ONLY as the HKDF IKM for session-key derivation (`server.rs:735/3823/3874`). All three sites must use this same accessor; changing the IKM rotates every derived session pubkey and strands live members. This is the one place that must be byte-identical to today.
- `management_nsec()` — signer for NIP-29 management (create/lock/put-user/put-admin/edit-metadata) per the real authority model. The backend key is the configured admin, so prefer `tenex_private_key.or(user_nsec)`. The current management reads become `management_nsec()` rewrites (`provider.rs:112/139/289/525` via the renamed provider field; `server.rs:2213/2665/2708`).
- `backend_nsec()` → `tenex_private_key.or(user_nsec)` — the daemon's own identity (added as admin to every group; matches `add` tags in the listener).
- `rpc_user_prompt` (`server.rs:2115`) stays operator-only (`user_nsec` directly) — a user prompt signed by the backend key would misattribute authorship.

Precompute on `DaemonState` (`server.rs:65-108`, populated in `run()` ~`:199`): `backend_keys: Option<Keys>` and `backend_pubkey: Option<String>` from `cfg.backend_nsec()`. This is what the listener matches `add` tags against and what gets added as admin.

### 2. Decouple group `h` from working-dir `project` — keep `project` = the group
Today the session record's `project` field IS the NIP-29 `h` used everywhere for routing (status `server.rs:3511`, chat `fabric/nip29/materializer.rs:119`, mentions, membership, peer state, tails). **Do NOT add a second routing key** (`group_h`) beside it — that would make every `.project` read a latent misroute. Instead **invert the seam**: let the session's `project` BE the child `h`, and add a separate `work_root` only where the directory is needed (cwd, tmux launch, resume, HKDF). Then all fabric publishing keeps using `project` unchanged.
- New env var `TENEX_EDGE_GROUP`. `tmux::spawn_agent` (`tmux.rs:803-836`) gains `group: Option<&str>`; when set, `open_agent_session` adds `-e TENEX_EDGE_GROUP=<child_h>` alongside `TENEX_EDGE_SPAWNED=1` (`tmux.rs:685-702`).
- Session-start hook (`src/cli/hooks.rs` ~`:374`) reads the env var like it reads `TMUX_PANE` and passes it as `"group"` in the RPC params.
- `SessionStartParams` (`server.rs:605-632`) gains `#[serde(default)] group: Option<String>`.
- `rpc_session_start` (`server.rs:657`): compute `work_root = project::resolve(cwd)` (the directory/repo slug) and `session_project = p.group.unwrap_or(work_root)`. Store `session_project` in the existing `project` column (so all downstream publishing routes to the child); add a `work_root TEXT` column (default = `project` for legacy rows) for cwd/tmux/resume. **Use `work_root` (not the child h) as the HKDF info-string** so the derived key is stable per repo+agent regardless of room and matches `session_ikm_nsec()` continuity.
- **Session resolution (critical, per codex):** `resolve_session` (`server.rs:494-509`) falls back cwd→project→latest-alive, which is ambiguous when a parent-project session and a subgroup session share a cwd. Subgroup sessions must be resolved by their **explicit session anchor** (`TENEX_EDGE_SESSION` / harness session id), not cwd. Verify the spawned harness exports its session id and that every session-behalf RPC (`chat write`, `who`, statusline, turn hooks) carries it; the cwd fallback must not silently bind a subgroup command to the parent room. tmux launch/resume/`project_abs_path` (`tmux.rs:607-617`) read `work_root`.

This inversion means almost no `.project` publish sites change. The audit shifts to: anything that resolves the *directory* from `project` must instead read `work_root` (tmux launch, resume, statusline cwd display).

### 3. Subgroup lifecycle builders (`src/fabric/nip29/lifecycle.rs`)
Add, mirroring existing builder + unit-test style:
- `group_create_subgroup(child_h)` — delegates to `group_create` (kind:9007).
- `group_lock_closed_with_parent(child_h, name, parent_h)` — kind:9002 with `["h",child_h]`, `["name",name]` (human display name), `["parent",parent_h]`, `["closed"]`, `["public"]`. (Issue body specifies `["parent","tenex-edge"]`; confirm exact arity against #2319.)

Add `util::child_group_id(name)` next to `slugify_host` (`src/util.rs:149`): `<slugify(name)>-<random8>` (8 hex chars from 4 random bytes; reuse `nostr_sdk`'s `rand`). Unit-test prefix/suffix/uniqueness.

### 4. Read + copy parent admins (`src/fabric/provider.rs`)
Extract the 39000/39001/39002 fetch+parse block in `open_project_with_progress` (`provider.rs:316-373`) into a reusable helper `fetch_group_state(group) -> (exists, roles, members)` / `fetch_group_roles(group)`, and have `open_project_with_progress` call it (pure refactor, one source of truth). For subgroup admin copy (first cut), the live role fetch can identify `admin` but not human-vs-agent-vs-session (`provider.rs:347`), so use an explicit **inclusion** set: copy a parent admin `pk` iff `pk ∈ whitelisted_pubkeys` OR `pk == pubkey(userNsec)` OR `pk == backend_pubkey` OR `pk` is one of the backend pubkeys explicitly targeted in this command **and** is a parent admin. **Defer** "known peer-backend" inference until backend discovery exists.

### 5. CLI + RPC
- `ProjectAction::CreateGroup { parent, name, agents: Vec<String> (--agent slug@backend, repeatable), message: Option<PathBuf> }` in `src/cli.rs:468-490`. Dispatch at `:631` already routes to `admin::project`.
- `admin::project` arm (`src/cli/admin.rs:5-72`): split each `--agent` on the last `@` into `(slug, backend)`; read `--message` file (bail if unreadable); call `daemon_call_async("project_create_group", json!({parent, name, agents:[{slug,backend}], brief}))`; print child `h`, display path, orchestration event id.
- Daemon RPC `"project_create_group" => rpc_project_create_group` (register at `server.rs:469`, handler beside `rpc_project_add` ~`:2650`). Steps:
  1. `relay_supports_subgroups()` gate (external) — bail loudly if unsupported.
  2. Resolve each `backend` token → backend pubkey (§6).
  3. `child_h = util::child_group_id(name)`.
  4. Sign with `management_nsec()` (bail if absent): publish `group_create_subgroup(child_h)` then `group_lock_closed_with_parent(child_h, name, parent)` (reuse `publish_group_management` benign-duplicate tolerance).
  5. Copy parent admins (§4) + add whitelisted + userNsec pubkey + backend_pubkey as admins (`group_put_admin`), deduped.
  6. `mark_group_owned(child_h)` + `ensure_subscription(child_h)`.
  7. Build + publish ONE kind:9 orchestration event into the **parent/coordination group** (§6).
  8. Locally invoke the orchestration handler directly on the just-built event (relays don't reliably echo to the publisher) — guarded by the durable processed-events table (§7), NOT `first_sight`. Return `{child_h, display_path, admins, orchestration_event_id}`.

### 6. `@backend` resolution + orchestration event
**Backend resolution (first cut): explicit pubkey/npub only.** A `backend` token is resolved via `resolve_pubkey_hex` (`server.rs:2768`); if it isn't a valid pubkey/npub, bail with a clear error. **Defer** host-label discovery: a kind:0 `te-role=backend` announcement is spoofable (a newer kind:0 can claim `edge1`) and is not trust-safe without a verification model, so it's explicitly out of the first cut. (The friendly `@edge1` form returns once a trusted host→pubkey mechanism exists; until then operators pass `slug@<npub>`.)

**Backend-wide orchestration delivery (critical gap, per codex):** `scope_filters` (`fabric/nostr_delivery.rs:70-73`) subscribes to kind:9 only by `#h == project`; only kind:1 mentions get a p-tag fallback (`:77-80`). A remote backend with no live session in the parent group would therefore **never receive** a parent-routed orchestration event. Add a durable, backend-wide subscription for **kind:9 p-tagged to `backend_pubkey`** (independent of any project scope), so every daemon receives orchestration events addressed to it regardless of group membership. This is what actually makes cross-device auto-start work.

**Orchestration event** — new `src/fabric/nip29/orchestration.rs`, built as a raw `EventBuilder` (NOT the ChatMessage encoder, which strips unknown tags):
```
kind:9, content = prose (from --message, else generated "@edge1: add research-lead. ...")
["h", <parent_h>]                 # routed into the coordination group (see decision below)
["te-op", "subgroup.add-agents.v1"]
["parent", <parent_h>]
["h-target", <child_h>]           # the child group id
["p", <backend_pk>] ...           # one per distinct target backend
["add", <backend_pk>, <role_slug>] ...
```
**Routing-`h` decision:** publish into the **parent** (`h = parent`), carry child in `h-target`. The issue's example lists both a child `h` and a `parent` tag, but a kind:9 has one routing `h`; routing into a closed *child* would hide it from backends that aren't members yet, breaking cross-device delivery. Parent-routing + the backend p-tag subscription (above) reaches the targeted backends. (Flagged for confirmation with the issue author / #2319.)

**Parser hardening (per codex):** before acting, require `event.h == parent`, exactly one `h-target`, and a valid signer/admin check (§7). Before spawning into `child_h`, **verify the child's `kind:39000` actually carries `parent == parent`** — otherwise a parent admin could direct joins/spawns into an arbitrary group id.

### 7. Orchestration listener (receive side, `src/daemon/server.rs`)
Hook in `handle_incoming` (`:3200-3230`) after `materialize`: if `kind==9` and `parse_orchestration(event)` returns `Some`, `tokio::spawn(handle_orchestration(...))` (async: relay fetches + publishes + spawn).
- **Idempotency (per codex):** do NOT reuse `first_sight` — it's an in-memory, bounded, restart-non-durable dedupe shared with the tail path, so an old orchestration event could respawn agents after a restart, and ordering could suppress a chat tail. Add a durable `processed_orchestration(event_id PRIMARY KEY, at)` table; skip events already recorded; record on successful handling.
- `parse_orchestration` reads **tags only**: require `["te-op","subgroup.add-agents.v1"]` (else `None` → prose ignored); require `event.h == parent` and exactly one `h-target`; extract `parent`, `child_h`, and all `["add",pk,role]`. Ignore body.
- `handle_orchestration`:
  1. **Authorize:** `fetch_group_roles(parent)`; require signer (`event.pubkey`) be an `admin` (accept child-admin too). Fail-closed on fetch error (ignore).
  2. **Filter** adds to those whose backend pubkey == `state.backend_pubkey`; return if none.
  3. **Verify child belongs to parent:** read child `kind:39000`, require its `parent == parent` (reject arbitrary group ids). `ensure_subscription(child_h)` + mark owned/membership cache.
  4. For each targeted `role_slug`: `identity::load_or_create(edge_home, role_slug, now)` → publish kind:0 (reuse `rpc_publish_profile` body, signed with role keys) → `nip29_add_member(child_h, role_pk)` (member, never admin). **Gate the spawn on a confirmed member add (per codex):** `nip29_add_member` is best-effort (`server.rs:891`); if it returns false (relay rejected), do NOT spawn — a live harness whose status/chat the relay rejects is worse than no harness; log and let a retry handle it. On success → `spawn_agent(state, role_slug, parent /*resolves work_root via parent's dir*/, args, Some(child_h) /*group override*/)`. The spawned session's session-start hook then adds the **session** pubkey as a child member via the §2 path (no precomputed session keys).

Keep pure cores (`parse_orchestration`, `is_authorized(roles, signer)`, `adds_for_backend(adds, backend_pk)`) as free functions for unit testing.

## Critical files
- `src/config.rs` — key split + `management_nsec()`/`backend_nsec()` accessors + tests.
- `src/daemon/server.rs` — `DaemonState` backend identity; `rpc_project_create_group` + dispatch; `rpc_session_start` `project = group h` + `work_root` split; backend p-tag subscription; `handle_incoming` listener hook; `processed_orchestration` idempotency.
- `src/fabric/nip29/lifecycle.rs` — subgroup create/lock-with-parent builders.
- `src/fabric/nip29/orchestration.rs` (new) — build/parse add-agents kind:9 + auth/filter helpers.
- `src/fabric/provider.rs` — `fetch_group_roles` refactor; subgroup admin copy.
- `src/tmux.rs` — `spawn_agent` group override → `TENEX_EDGE_GROUP`; `src/cli/hooks.rs` forwards env into session-start.
- `src/cli.rs`, `src/cli/admin.rs` — `CreateGroup` variant + handler.
- `src/util.rs` — `child_group_id`.

## Verification
- **Unit tests** (mirror `lifecycle.rs:70-156` style): subgroup builders carry `h`/`name`/`parent`/`closed`/`public` and NOT `private`; `child_group_id` shape/uniqueness; orchestration build↔parse round-trip; `parse_orchestration` returns `None` for plain prose, unknown `te-op`, mismatched `event.h != parent`, or missing/duplicate `h-target`; config split (only `tenexPrivateKey` set → all three accessors resolve to it; both set → `session_ikm_nsec` follows the old collapse while `management_nsec`/`backend_nsec` prefer the backend key); `is_authorized` / `adds_for_backend` pure helpers.
- **End-to-end across two daemons** (isolate via `TENEX_EDGE_HOME`/`TENEX_CONFIG`, distinct `tenexPrivateKey` each): create parent → `project create-group --parent P --name "subgroup support" --agent research-lead@<A-pk> --agent testing-lead@<B-pk> --message ./brief.md`. Assert: kind:9 with `te-op` on relay; child `h` == `subgroup-support-<8>`; child `kind:39000` `parent=P` on read-back; child `kind:39001` has parent human/backend admins, no agent/session pubkey as admin; A spawns `research-lead` with `work_root`=parent repo but its session `project`=`child_h` so status/chat route to `child_h` (child `kind:39002` lists the session pubkey; a `chat write` from that pane resolves by explicit session id and routes to the child, not the parent); B (no live parent session) auto-starts `testing-lead` from the relayed kind:9 alone via the backend p-tag subscription. Negative: non-admin signer → no spawn; prose-only kind:9 → ignored; `h-target` whose child `parent != P` → rejected before spawn; redelivered/post-restart duplicate event → no second spawn.
- **Negative:** kind:9 add-agents signed by a non-admin → no spawn; plain prose kind:9 ("please add testing-lead") → ignored.
- Optionally surface `parent` in `rpc_project_list` (parse it in `Nip29Materializer::materialize_group_metadata`, `fabric/mod.rs:99`) to render `parent > name` and give step-5 an automated hook.

## First cut vs deferred
**First cut proves:** explicit backend pubkey (`slug@npub`); parent-routed structured kind:9 + backend p-tag subscription; durable idempotent receiver; child `h` for all session publications (no key rotation); parent-admin copy of the known human/backend set; spawn gated on confirmed member add.
**Deferred:** host-label `@edge1` discovery (needs a trust-safe host→pubkey mechanism); "known peer-backend" admin-copy inference; `project list` hierarchy rendering; any UI niceties.

## Open items to confirm
1. Orchestration routing-`h`: parent-routed (recommended) vs literal child-`h` from the issue example.
2. Exact `parent` tag arity per #2319 (`["parent", id]` vs `["parent", id, relay]`).

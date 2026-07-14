# tenex-edge — Fabric Architecture Implementation Ladder

Companion to [fabric-architecture.md](fabric-architecture.md). This file owns the phased refactor ladder and validation commands; the architecture principles remain in the canonical fabric document.

## 6. Implementation plan

The refactor should be done in behavior-preserving slices. The first provider is
not a new fabric; it is today's legacy-tag + NIP-29 behavior pulled behind the new
seams. Do not start by deleting the current `Codec`/`Transport`/`Store` paths.
Add the new read model, dual-write where necessary, cut readers over, then delete
legacy access once tests prove the projections are authoritative.

### Guardrails

- Existing host adapters stay thin and keep their current CLI/RPC surface:
  `who`, `channel read/send`, `channel list --all-workspaces/init/edit`,
  `channel create/list/join/leave/switch/add`, `mgmt agent`,
  `harness hook`, `harness statusline`, `publish`, `launch`, `sessions`, and
  `mcp`.
- The daemon remains the only SQLite writer. New provider code must not open its
  own `rusqlite::Connection`.
- Existing behavior is the regression oracle: same-machine local delivery,
  targeted session mentions, NIP-29 group create/lock/add, project metadata cache,
  relay-authoritative membership routing, and startup mention fetch must still work.
- Keep legacy tables during cutover. The plan below adds canonical tables and
  backfills/dual-writes before any old table is removed.

### Phase 0 - freeze current behavior

Before moving code, add tests around the behavior that will be extracted.

Files:

- `tests/daemon_integration.rs`
- `tests/daemon_mechanics.rs`
- `tests/e2e_transport.rs`
- `src/state.rs` unit tests
- `src/runtime.rs` unit tests

Coverage to pin:

1. `channel send` to a hosted sibling session inserts one inbox row using the
   signed event id for directed delivery, and relay echo/fetch does not duplicate
   it.
2. A targeted session mention reaches only the target session; an untargeted
   mention reaches alive sessions for that recipient agent/project only.
3. `handle_incoming` applies 39000 metadata and 39002 membership snapshots
   idempotently.
4. `who` and hook-injected turn context are rendered from store state only.
5. Delivered profile events enter `profiles`; local allow/block state is not
   part of the active NIP-29 path.
6. `fetch_mentions_into_inbox` catches stored kind:1 mentions after startup.

Done when: these tests fail if `handle_incoming`, `route_mention_into`, or
`resolve_recipient` regress during extraction.

### Phase 1 - add canonical read-model schema

Extend `src/state.rs` without changing readers yet.

Add durable ids and origins:

- `projects(project_id TEXT PRIMARY KEY, display_slug TEXT NOT NULL, about TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)`
- `project_origins(project_id TEXT NOT NULL, fabric TEXT NOT NULL,
  provider_instance TEXT NOT NULL, native_project_key TEXT NOT NULL,
  UNIQUE(fabric, provider_instance, native_project_key))`

Add canonical communication rows:

- `messages(message_id TEXT PRIMARY KEY, thread_id TEXT NOT NULL,
  channel_h TEXT NOT NULL, author_pubkey TEXT NOT NULL,
  body TEXT NOT NULL, created_at INTEGER NOT NULL, direction TEXT NOT NULL,
  sync_state TEXT NOT NULL, native_event_id TEXT, error TEXT)`
  - `author_pubkey` is the return address. Runtime incarnations are process
    locators, so relay replay and process replacement cannot change who owns a
    message. The reply handle stays a read-side derivation. `inbox` is delivery
    state only.
- `message_recipients(message_id TEXT NOT NULL, recipient_pubkey TEXT NOT NULL,
  delivered_at INTEGER, PRIMARY KEY(message_id, recipient_pubkey))`
- `inbound_quarantine(native_event_id TEXT PRIMARY KEY, project_id TEXT, reason
  TEXT NOT NULL, raw_envelope TEXT NOT NULL, created_at INTEGER NOT NULL)`

Normalize membership:

- `membership(project_id TEXT NOT NULL, pubkey TEXT NOT NULL, role TEXT NOT NULL,
  admitted_at INTEGER NOT NULL, revoked_at INTEGER, source TEXT NOT NULL,
  updated_at INTEGER NOT NULL, PRIMARY KEY(project_id, pubkey))`

Accessors to add first:

- `ensure_project_origin(fabric, provider_instance, native_project_key, display_slug) -> project_id`
- `project_id_for_origin(fabric, provider_instance, native_project_key) -> Option<String>`
- `record_message(...) -> message_id`
- `mark_message_sync_state(message_id, sync_state, error)`
- `add_message_recipient(message_id, recipient_pubkey, delivered_at)`
- `admit_member(project_id, pubkey, role, source, ts)`
- `revoke_member(project_id, pubkey, ts)`
- `is_member_at(project_id, pubkey, ts) -> MembershipDecision`
- `quarantine_inbound(native_event_id, project_id, reason, raw_envelope)`
- `replay_quarantine(project_id/pubkey filter) -> Vec<RawEnvelopeRef>`

Backfill rules:

- For every existing `relay_channels.channel_h`, `sessions.channel_h`,
  `relay_status.channel_h`, and `relay_channel_members.channel_h`, create a
  project with origin `(fabric='nip29', provider_instance=<relay set hash>,
  native_project_key=<slug>)`.
- Mirror `relay_channel_members` into `membership` with source by role.
- Historical kind:9 `relay_events` are idempotently backfilled into `messages`
  when the store opens. New inbound/outbound chat dual-writes immediately. When
  dual-writing, preserve the event author's pubkey as the durable return address.

Done when: the new tables can be created on an existing `state.db`, backfilled
idempotently, and queried without changing CLI output.

### Phase 2 - split StoreReader and StoreWriter

Keep the concrete `Store` type, but narrow how callers use it.

Add read-facing methods for host/RPC code:

- `list_projects_read_model()`
- `channel_meta_read_model(channel_id or slug)`
- `list_agents_read_model(project)`
- `list_presence_read_model(project)`
- `list_status_read_model(project)`
- `undelivered_messages_for_session(session_id)`

Add write-facing methods for provider/materializer code:

- `materialize_profile(...)`
- `materialize_presence(...)`
- `materialize_status(...)`
- `materialize_membership_snapshot(...)`
- `materialize_inbound_message(...)`
- `materialize_outbound_message(...)`
- `mark_outbound_accepted/echoed/failed(...)`

Then move read assembly behind those methods:

- `rpc_who` stops depending on `relay_profiles`/`relay_status` layout directly.
- `assemble_turn_start_context` and `assemble_turn_check_context` read through the
  read model.
- `rpc_project_list` prefers local `projects` rows, using relay fetch only as a
  materialization refresh.

Compatibility rule: if a read-model row is absent during rollout, readers may
fall back to legacy tables. That fallback must be temporary and covered by a
TODO naming the phase that removes it.

Done when: every reader has a read-model method to call, even if some methods
still bridge to legacy tables internally.

### Phase 3 - extract raw Nostr delivery from the codec

Today `Codec::filters` mixes wire shape and subscription mechanics. Split it
without changing the daemon subscription behavior.

New module shape:

- `src/fabric/mod.rs`
- `src/fabric/nostr_delivery.rs`
- `src/fabric/nip29/wire.rs`
- `src/fabric/nip29/materializer.rs`
- `src/fabric/nip29/lifecycle.rs`

Types:

```rust
pub enum RawEnvelope {
    Nostr(nostr_sdk::Event),
}

pub struct Scope {
    pub authors: Vec<String>,
    pub project: Option<String>,
}

pub trait Delivery {
    fn name(&self) -> &'static str;
    async fn publish(&self, envelope: RawOutboundEnvelope) -> Result<NativeId>;
    async fn publish_checked(&self, envelope: RawOutboundEnvelope) -> Result<NativeId>;
    async fn subscribe(&self, scope: Scope) -> Result<()>;
    async fn fetch(&self, scope: FetchScope) -> Result<Vec<RawEnvelope>>;
    fn notifications(&self) -> RawEnvelopeStream;
}

pub trait NostrEventCodec {
    fn encode(&self, event: &DomainEvent) -> Result<nostr_sdk::EventBuilder>;
    fn decode(&self, envelope: &RawEnvelope) -> Option<DomainEvent>;
}
```

Implementation steps:

1. Keep filter construction in `NostrDelivery::subscribe(scope)` /
   `scope_filters(scope)`, not in the codec.
2. Keep `Transport` as the private implementation detail of `NostrDelivery`.
3. Keep `Nip29WireCodec::encode/decode` under `src/fabric/nip29/wire.rs` and
   expose the shared trait as Nostr-event-specific while it returns
   `nostr_sdk::EventBuilder`.
4. Remove the old `Codec` trait/module; NIP-29 is the active wire provider.

Done when: `resubscribe` no longer calls `state.codec.filters(&scope)`; it asks
delivery to subscribe to `Scope`, while decode remains in the provider codec.

### Phase 4 - extract the materializer

Move `handle_incoming` out of `src/daemon/server.rs` into a materializer owned by
the provider.

Materializer input:

- raw envelope
- hosted local pubkeys
- current time
- constrained `StoreWriter`

Materializer output:

- optional rendered/tail domain event
- mention wake signal
- quarantine/replay request
- sync-state reconciliation event

Extraction order:

1. Move 39000 handling to `Nip29Materializer::materialize_group_metadata`.
2. Move 39002 handling to `Nip29Materializer::materialize_membership_snapshot`.
3. Move profile materialization to `Nip29Materializer::materialize_profile`.
4. Move presence/status upserts to `Nip29Materializer`.
5. Move mention routing from `runtime::route_mention_into` into
   `materialize_inbound_message`, while leaving a thin compatibility wrapper for
   existing tests.
6. Move startup mention fetch through the same materializer path; it should not
   have a separate decode/route implementation.

Admission behavior:

- Nostr admission uses `event.pubkey` as the actor identity. The self-asserted
  `["agent", pk, slug]` wire tag has been removed from all events (Presence,
  Status, Activity, Mention); slug is resolved from the signer's kind:0 Profile
  by pubkey. Routing and admission never consult a tag — authorization is by
  signer pubkey + group membership only.
- If membership for the project is hydrated and sender is not admitted, drop the
  chat before it reaches `relay_events`, `messages`, tail, or inbox routing.
- If membership is not hydrated yet, quarantine. Replay after both 39001 admins
  and 39002 members snapshots hydrate for the channel.
- Presence/status/current roster use current membership; admitted messages remain
  historical after revocation.

Done when: `server.rs::handle_incoming` is a small dispatch to
`provider.materialize(raw_envelope)`, and every inbound path shares the same
materialization code.

### Phase 5 - introduce FabricProvider

Once delivery, provider codec, materializer, and lifecycle have concrete homes, add
the top-level provider.

Provider API shape (pseudo-Rust; implement first with a concrete
`Nip29Provider`, or use boxed futures / `async_trait` before storing it
behind `dyn`):

```rust
pub trait FabricProvider {
    fn name(&self) -> &'static str;
    async fn open_project(&self, project_slug: &str, agent_pubkey: &str) -> Result<()>;
    async fn send(&self, intent: SendIntent) -> Result<OutboundReceipt>;
    async fn set_status(&self, status: StatusIntent) -> Result<OutboundReceipt>;
    async fn subscribe_project(&self, project_id: &str) -> Result<()>;
    async fn catch_up_mentions(&self, session: &SessionRecord) -> Result<usize>;
    fn materialize(&self, envelope: RawEnvelope) -> Result<MaterializationOutcome>;
}
```

The first implementation is `Nip29Provider`:

- Lifecycle wraps the current `ensure_group_and_membership`.
- Delivery wraps `Transport`.
- Wire codec owns the current NIP-29 event shape.
- Materializer owns metadata, membership, profile, presence, status, messages,
  recipients, quarantine, and tail output.

Daemon changes:

- `DaemonState` holds one active provider (`Nip29Provider` at first; a
  provider enum or object-safe boxed trait later) instead of exposing separate
  `transport + codec` fields to RPC code.
- `spawn_demux` receives raw envelopes from provider delivery and calls
  `provider.materialize`.
- `rpc_session_start` calls `provider.open_project(...)` before spawning the
  engine.
- `ensure_subscription` becomes `provider.subscribe_project(project_id)`.
- `fetch_mentions_into_inbox` becomes `provider.catch_up_mentions(&rec)`.

Done when: daemon RPC behavior is unchanged, but fabric-specific code is no
longer in `server.rs` except provider construction.

### Phase 6 - move outbound writes to read-model messages

Refactor `rpc_send_message` around an explicit send intent.

Steps:

1. Resolve the sender pubkey and recipient pubkeys using read-model accessors.
2. Build `SendIntent { from_pubkey, to_pubkeys, project_id, body }`; runtime
   locators may be selected for immediate local delivery but are not persisted.
3. Provider signs first, returning the native event id for the exact event that
   will be published or retried.
4. Store inserts/updates:
   - `messages.sync_state='pending'` for explicit local optimistic UX before
     relay acceptance.
   - `messages.sync_state='accepted'` when checked publish confirms relay OK.
   - `messages.sync_state='failed'` with `error` on rejection/timeout.
5. Same-daemon hosted recipients are delivered by inserting message recipient
   rows using the final native id. Echo/fetch is idempotent through
   `native_event_id` + recipient uniqueness.
6. `inbox` becomes either:
   - a compatibility projection over `messages` + `message_recipients`, or
   - a legacy table dual-written until turn injection is cut over.

Done when: `inbox` and turn-start context can render from
canonical message rows without losing the old delivered/seen semantics.


### Phase 8 - remove legacy coupling

Only after the cutover tests pass:

- Remove `Codec::filters` from the public seam.
- Move NIP-29 group builders out of `src/fabric/nip29/wire.rs`.
- Delete direct fabric handling from `daemon/server.rs`.
- Remove direct reader dependence on `relay_profiles`, `relay_status`, `inbox`,
  and `relay_channels`; keep tables only if they are compatibility views or
  deliberately retained storage.
- Replace `SubScope` with `Scope` everywhere.
- Update docs/wiki pages after the source doc is correct.

Done when: adding an MLS/A2A provider means implementing provider capabilities,
not editing reader code or daemon routing logic.

### Validation ladder

Run this after each phase, broadening only when the phase touches more surface:

1. CI-safe gates: `just fmt-check`, `just loc-check`, `just lint`, and
   `just test-unit`.
2. `just test` when local relay dependencies are available (`nak` for plain
   relay tests; croissant via `$NIP29_RELAY_BIN` for NIP-29 group tests).
3. `cargo test --test daemon_mechanics` for daemon/socket lifecycle changes.
4. `cargo test --test daemon_integration` for daemon RPC, membership, routing,
   and delivery changes.
5. `TE_RELAY=<relay> cargo test --test relay_probe -- --ignored --nocapture`
   only when validating public-relay shared-connection AUTH behavior.
6. `TE_NIP29_RELAY=<relay> cargo test --test nip29_probe -- --ignored --nocapture`
   only when validating live NIP-29 lifecycle or membership materialization.
7. `TE_NIP29_RELAY=<relay> cargo test --test seed_validation -- --ignored --nocapture`
   only when seeding a reader app with a complete validation session.
8. Manual smoke:
   - start two sessions in the same project;
   - human `tenex-edge who` and agent `tenex-edge my session`;
   - send a chat mention with `tenex-edge channel send --message "@<agent> ..."`;
   - verify the target receives the mention through its host hook or pty
     delivery path;
   - verify no duplicate after a fetch or hook-injected turn context;
   - edit project metadata and confirm local read-model update.

---

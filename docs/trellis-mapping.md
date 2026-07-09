# Trellis Mapping for Derived Resources

Status: audit map for issue #203, part of epic #202. This document maps the
current tenex-edge derived-resource surfaces to Trellis primitives before any
effect path changes.

## Boundary And End-State

Trellis is the private reconciliation engine. tenex-edge owns observations and
effects; Trellis owns decisions. Observed facts enter a host-owned input journal,
a Trellis transaction computes the new derived shape, and the host applies the
resulting inert frames or commands. Effect results then return as new observed
facts. Trellis does not read SQLite, watch processes, call LLMs, sign events,
publish to relays, or inject hook text.

The long-term shape is one journal-fed graph, with SQLite acting as durable log
and projection store. Later slices can keep using existing tables as the input
adapter while moving toward this loop:

1. observed fact enters the journal;
2. the daemon converts journal facts into `InputNode<T>` updates;
3. Trellis derives sessions, status, who/context views, outbox intent, and
   subscription intent;
4. the daemon applies DB-write frames, relay commands, and hook/output frames;
5. apply success or failure enters the journal as the next observed fact.

Host facts that may enter the graph include `SessionStarted`, `TurnStarted`,
`TranscriptWindowCaptured`, `DistillCompleted`, `TurnEnded`,
`RelayEventObserved`, `RelayPublishAccepted`, `ProcessExited`, `ClockTick`, and
configuration changes. Trellis may derive projection writes for `sessions` and
`relay_status`, current activity, kind:30315 status frames, `who` and hook
context output frames, relay subscription open/close intent, stale-status
filtering, and causal explanation.

Large payloads stay outside the graph. Full transcripts, raw Nostr event bodies,
long relay history, and logs should be represented by stable pointers, hashes,
small summaries, or indexed facts before becoming Trellis inputs.

Use the Trellis vocabulary this way:

- `InputNode<T>`: canonical host facts such as `sessions`, `session_channels`,
  `identities`, selected relay-cache snapshots, `now`, and `seen_cursor`.
- `DerivedNode<T>`: deterministic computations over explicit `DependencyList`s,
  such as subscription coverage, status payloads, and fabric context views.
- `CollectionNode<K, V>` or `CollectionNode<K, ()>`: desired sets/maps that
  produce `MapDiff` or `SetDiff` on each transaction.
- `ResourcePlan<C>`: inert open, close, replace, or refresh commands. The host
  applies commands with the transport/provider.
- `MaterializedOutput<T>` and `OutputFrame`: materialized status and hook-context
  payloads. The host publishes or injects frames after commit.
- "Receipt" in this repo means the `TransactionResult<C>` plus audit queries
  such as `why_changed`, `why_resource_command`, `why_output_frame`,
  `dependency_path`, and `scope_resource_inventory`.

Shadow mode for later slices should compare desired state, not raw command
streams. For resources, compare desired keys and owners. For outputs, compare
materialized payloads. The existing path stays authoritative until the shadow
path earns promotion. Shadow mode has no split-brain risk because Trellis does
not apply effects.

Promotion has a stricter rule: each surface must have exactly one decider and
one writer. When a slice promotes a surface, every existing writer for that
surface must either become a journal input or be replaced by a Trellis-produced
write intent in the same change. Running some subscription, status, or hook
paths through Trellis while other paths still write directly is forbidden.

## Retrospective Receipts

Slice #210 should extend the same transaction path, not recreate a parallel
audit system. Every projection write, resource command, and output frame should
carry enough artifact metadata for `tenex-edge debug explain <handle>` to recover why
the effect happened:

- Trellis transaction id and input-journal range or cursor.
- Stable surface key, such as status session id, hook call id, subscription key,
  or outbox event id.
- Hashes or pointers for large artifacts that stayed outside the graph.
- The `TransactionResult` audit handle that can answer `why_changed`,
  `why_resource_command`, `why_output_frame`, and dependency-path questions.

LLM calls are host provenance, not graph computation. A `llm_calls` ledger should
record model, provider, system prompt identity, transcript-slice pointer,
request/response pointers or hashes, and the resulting `DistillCompleted` input
fact. Trellis should depend on the distill fact, while `explain` can join back
to the ledger when a status or hook output was caused by that distillation.

## Shared Inputs

The local, non-rebuildable tables are the main owned truth:

- `sessions`: session id, agent pubkey/slug, active channel, liveness,
  turn state, `seen_cursor`, title, and activity (`src/state/schema.rs`).
- `session_channels`: passive/active channel membership per session
  (`src/state/schema.rs`).
- `identities`: per-session derived pubkey, channel, native id,
  and alive bit (`src/state/schema.rs`).
- `inbox` and `outbox`: local delivery and publish ledgers. These are not
  computed relay state.

Relay-backed tables are materialized input snapshots, not local owned truth:

- `relay_channels` and `relay_channel_members` describe channel metadata and
  membership as materialized by the fabric provider.
- `relay_status` is the read cache for observed kind:30315 status rows. It is
  not the local source of what this daemon should publish.
- `messages` and `relay_events` provide read-side chat history for context.

Additional non-SQL inputs:

- `DaemonState.subscribed_projects`, currently an in-memory pin set for
  subscription coverage.
- local durable agent pubkeys from edge-home agent keys.
- live transient session pubkeys from `DaemonState.session_keys`.
- the backend pubkey.
- `now`, status TTL, heartbeat cadence, and `seen_cursor`.
- the nondeterministic distill result, which enters as a write to
  `sessions.title`, `sessions.activity`, and `sessions.last_distill_at`.

## Relay Subscriptions

Current shape:

- `ensure_subscription` is add-only: it records the channel in
  `subscribed_projects`, asks `SubscriptionRegistry::add_channel` for narrow
  `PlannedReq`s, and applies opens through `Transport::subscribe_with_id_to`.
- `build_entity_coverage` derives `EntityCoverage { channels_h,
  group_state_d, addressed_pubkeys_p }` from subscribed projects, per-session
  derived pubkeys, membership/admin rows, alive sessions' joined channels, live
  transient session keys, and the management-key pubkey.
- `SubscriptionRegistry` is already a pure planner. It holds aggregate and
  narrow coverage and returns `PlannedReq { id, filter }`; network I/O happens
  only when the daemon applies the plan.
- There are three aggregate REQ roles: all `#h` chat/status/long-form, all `#p`
  chat/long-form, and all group-state `#d` metadata/admin/member subscriptions.
- `rpc_channels_join` opens coverage before inserting `session_channels`.
  `rpc_channels_leave` and `rpc_channels_switch` mutate membership/session
  state but do not close relay subscriptions.
- `close_subs` and `SubscriptionRegistry::compact` are present but dead-code
  paths. There is no remove-channel/refcount path today.

Trellis mapping:

- Inputs:
  - `subscribed-projects`: daemon-pinned channel set.
  - `local-pubkeys`, `identity-pubkeys`, `live-session-pubkeys`,
    `backend-pubkey`.
  - `alive-sessions`: session id, active channel, created time.
  - `joined-channels`: `(session_id, channel_h, joined_at)`.
  - `channel-membership`: materialized membership/admin rows.
- Derived node:
  - `subscription-coverage`: the `build_entity_coverage` result.
- Collections:
  - `desired-reqs`: a map keyed by structured resource identity, with values
    holding the host command payload needed to subscribe.
  - Resource keys should be structured segments such as
    `["nostr-req", "h", channel_h]`, `["nostr-req", "gstate", channel_h]`,
    or aggregate-role equivalents. Do not parse flattened subscription-id
    strings as product identity.
- Resource plan:
  - A `map_resource_planner` should turn desired REQ diffs into `Open`,
    `Close`, and `Replace` commands.
  - The host executor maps opens to `subscribe_with_id_to` and closes to
    `unsubscribe`.
- Scopes:
  - A daemon/root scope owns aggregate coverage.
  - Session scopes own joined-channel interest.
  - Shared channel REQs coalesce under the same resource key and close only
    when the last owning scope leaves.

This is the input map needed by #204. Promotion in #205 should delete the
bespoke add-only path after close-on-last-owner is validated.

## kind:30315 Status

Current shape:

- Local publish intent lives on the `sessions` row: `agent_pubkey`,
  `agent_slug`, `channel_h`, `alive`, `last_seen`, `working`,
  `turn_started_at`, `last_distill_at`, `title`, and `activity`.
- Status `h` tags are derived from `session_channels`, with `sessions.channel_h`
  as a fallback. Both current status builders sort and dedupe this set.
- The bound `identities` row is the identity/signing input for a session; the
  daemon falls back to the base session row if the binding is unavailable.
- There are two status builders today. `runtime::status_for` clears `activity`
  while idle and preserves `rel_cwd`; `status_publish::status_from_session`
  copies activity even when idle and leaves `rel_cwd` empty.
- There are also two publish paths. The per-session engine signs a status and
  enqueues raw JSON into `outbox`; the daemon heartbeat publisher bypasses
  `outbox`, calls `provider.set_status`, and mirrors the accepted status into
  `relay_status`.
- `relay_status` is a materialized cache, one row per `(pubkey, session_id,
  channel_h)`, live only while `expiration >= now`.
- Session death and session end mark local rows dead. The final published
  kind:30315 ages off by NIP-40 expiration; no final or expired status is
  published today.
- Channel leave deletes the `session_channels` row and membership, but does
  not retract or correct the old status row for that channel.

Trellis mapping:

- Inputs:
  - `session-local`: the `sessions` status-relevant columns.
  - `session-identity`: bound identity/pubkey/signing selection.
  - `session-channel-set`: active plus passively joined channels.
  - `now`, status TTL, and explicit heartbeat/TTL-refresh tick.
  - `distill-result`: nondeterministic title/activity result, written as an
    input update to `sessions`.
- Derived node:
  - `status-payload/<session>`: one deterministic replacement for both current
    builders. It decides busy/idle text, activity clearing, h-tags, expiration,
    host, rel-cwd, and agent identity from declared inputs.
- Materialized output:
  - `status-frame/<session>` produces the publish payload.
  - The host signs and applies the frame through one publish executor. Keeping
    `outbox` as the durable executor is the natural current boundary.
- Collections:
  - `desired-status-frames`: live status payloads keyed by session id.
  - Optional teardown collection for closing/expired status commands when a
    session dies or a channel leaves.
- Scopes:
  - Per-session status scope owns the replaceable kind:30315.
  - Channel membership changes are inputs that recompute the h-tag set.
  - Closing a session scope must produce the deterministic teardown required by
    #207 instead of relying on passive TTL expiry.

This map supports #206 and #207. The important invariant is one derived status
payload and one host-applied output path; the current duplicate builders and
publishers should not be preserved as separate Trellis surfaces.

## Hook/Fabric Context Snapshot

Current shape:

- The daemon `turn_start` RPC sets `sessions.working` and `turn_started_at`,
  then calls `assemble_turn_start_context`.
- `assemble_turn_start_context` reads joined channels, claims pending inbox
  rows, computes first-turn ambient history, calls `render_fabric_context`, and
  advances `sessions.seen_cursor` after rendering.
- `turn_check` uses a compare-and-swap on `seen_cursor` so only one concurrent
  PostToolUse hook renders the delta. It then calls
  `assemble_turn_check_context`.
- `render_fabric_context` builds a `FabricView` with `build_view` and renders it
  as XML-like text.
- `build_view` reads project/channel metadata, joined channels, unjoined sibling
  channels, invitable agents, channel members, live status rows, and chat rows.
- `member_rows` and `presence_rows` use `relay_channel_members` plus
  `relay_status`, filtered by `now` and `cursor`.
- `message_rows` reads chat history after the cursor/window and merges forced
  inbox messages so direct mentions appear even when they are not in the relay
  history window.
- `turn_start_audit` and `turn_check_audit` hand-build an explanation in
  parallel. The audit's `awareness_json` scopes to the active channel plus
  direct subchannels, while the renderer scopes to `list_session_joined_channels`.
  That makes the audit an approximation rather than the render's dependency
  trace.

Trellis mapping:

- Inputs:
  - `session-viewer`: session id, current channel, created time, self identity,
    and `seen_cursor`.
  - `joined-channels`: active/passive channel set.
  - `channel-metadata`: `relay_channels` rows.
  - `channel-members`: `relay_channel_members` rows.
  - `live-status`: `relay_status` rows plus explicit `now`.
  - `chat-history`: `messages` or `relay_events` rows relevant to the joined
    channels and cursor/window.
  - `pending-inbox`: claimed or forced direct-mention rows.
  - `invitable-agents`: edge-home agent inventory.
  - `warnings`: host-detected read failures and membership warnings.
- Derived node:
  - `fabric-view/<session>`: deterministic `FabricView` over declared inputs.
- Collections:
  - `view-channels`, `view-members`, `view-presence`, `view-messages`,
    `view-unjoined-channels`, and `view-agents`.
  - Deltas are collection diffs keyed by stable row identity, not by rendered
    text.
- Materialized output:
  - `fabric-context/<session>` renders the view to the hook text frame.
  - Given the same inputs, including `now` and `seen_cursor`, the frame should
    replay byte-for-byte.
- Scopes:
  - A per-session view scope owns cursor-scoped context output.
  - Cursor advancement is a host-side input/output boundary: the graph receives
    the cursor used for the frame, and the daemon advances `sessions.seen_cursor`
    only after deciding the frame was consumed.

After this mapping, `HookCallLog` can remain a transport for the
`TransactionResult` audit data if useful, but `awareness_json` should not remain
a separately maintained explanation. The receipt should be `why_changed` for
the view/output itself.

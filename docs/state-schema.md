# State schema and upgrades

`state.db` is daemon-owned local persistence. It is not a cache or replica of
Nostr. NMP retained observations are the sole authority for current groups,
profiles, statuses, events, messages, recipients, and reactions.

Installing a newer binary must not create an operator-visible database step.
After taking the daemon startup lock and before serving requests, the store runs
every stamped migration from the installed schema to the binary's current
schema. Each migration is a one-way SQLite transaction: it preserves current
product-local state, removes the superseded schema in the same commit, and
leaves no runtime dual-read, alias, or compatibility path.

The migration table is compile-time-sized from `SCHEMA_VERSION`. Bumping the
version without adding the next contiguous migration does not compile. Tests
start from production-shaped deployed schemas and verify preservation through
the complete chain to current. A malformed source schema fails before its
version or tables are changed.

## Current schema: 22

Schema 22 removes all relay-derived persistence:

- `relay_channels`, `relay_channel_members`, and
  `relay_channel_member_sets`;
- `relay_profiles`, `relay_status`, and `relay_status_sets`;
- `relay_events` and `relay_reactions`;
- `messages` and `message_recipients`.

The current schema validator requires those tables to be absent. Mosaico does
not rebuild them after startup. Relay views repopulate only from active NMP
observations and disappear when NMP removes them or their observation closes.

The migration preserves exactly two small satellites from the former event and
message tables:

| Table | Durable local fact | Explicitly does not store |
|---|---|---|
| `nmp_event_arrivals` | Event id plus host-local monotonically increasing arrival sequence | Event kind, author, tags, content, timestamp, sources, or replacement state |
| `message_attachments` | Event id plus verified directory containing downloaded files on this host | Message content, recipients, attachment tags, URLs, or relay metadata |

The arrival sequence is a local route fence. It lets automatic ambient context
distinguish events observed before and after a session joined without copying
the event. The attachment directory is a local filesystem binding joined onto
an NMP-owned message while that message is present.

The remaining tables are product-local authority:

| Class | Tables |
|---|---|
| Sessions and local identity | `sessions`, `session_coaching`, `session_locators`, `session_signers`, `handle_leases`, `mcp_actor_aliases` |
| Routes and reconciliation | `session_channels`, `session_standing`, `workspace_roots`, `channel_resolution_intents` |
| Delivery and execution | `inbox`, `event_claims`, `native_turn_attempts` |
| Operational evidence | `channel_readiness_attempts`, `receipts` |

`session_channels` stores durable channel affinity plus signed-time and
arrival-sequence join fences. `session_standing` stores local desired standing
and cleanup progress. Neither table is evidence of current relay membership;
that comes only from the current NMP group observation.

The `inbox` is exact-recipient delivery state, not ambient history. Only an
observed message explicitly p-tagging a daemon-owned pubkey may create a pending
row. Publish acceptance alone creates no inbox work. The row remains claimable
when it predates registration or its route is later removed because local
delivery belongs to the exact identity, independently of ambient membership.

Completed offline-mention claims are compact durable tombstones keyed by event
and exact recipient. They do not expire: NMP may redeliver an event after
reconnection, and recovery or process launch must remain idempotent. The
tombstone does not keep the relay event current or make it readable after NMP
removes it.

Runtime endpoint locators carry their owning generation. PTY supervisor
attachment epochs and exit reports fence late callbacks, while persisted idle
deadlines let restart reconciliation continue the same headless-idle policy.
Only explicit forget/revoke changes recovery to `revoked` and removes the local
signer, routes, and locators after process termination is confirmed.

Launch admission facts are immutable for a runtime generation. A later hook may
update `claimed_harness`, but cannot reclassify a launch-owned
`observed_harness`, `admitted_preset`, `admitted_transport`, or
`endpoint_provenance`. Delivery resolves the exact locator keyed by the stored
observed harness and admitted transport; it never rereads mutable agent or
preset configuration to rediscover a live runtime's transport.

## Migration history

These entries describe the source shape of older deployments. They are not
current read contracts.

- **Schema 21** added `session_coaching`, a generation-scoped ledger of
  progressive agent guidance already emitted.
- **Schema 24** replaced the historical launch-config admission field with the
  selected preset name. Harness and transport are independent admission facts.
- **Schema 23** admitted Pi's native RPC transport and locator.
- **Schema 20** removed the redundant historical `messages.direction` field.
- **Schema 19** added a verified attachment directory to the historical
  `messages` projection. Schema 22 moves only non-empty directories into
  `message_attachments` before dropping that projection.
- **Schema 18** replaced each session's single current-channel pointer with
  durable multi-channel routes and arrival-sequence join fences, simplified
  standing to `member` or `absent`, and added the then-current status
  projection.
- **Schema 17** moved host capability and workspace discovery into complete
  kind:0 profile snapshots. The historical profile cache is removed by schema
  22; current profile state is read from NMP.
- **Schema 16** removed transcript paths and the explicit-chat publication
  marker after transcript auto-publication was removed.
- **Schema 15** added cumulative working time to session lifecycle state.
- **Schema 14** added `native_turn_attempts`, the daemon-owned operational
  ledger for generation-fenced native RPC turns. Open attempts reconcile to
  `unknown_reconciled` after restart; uncertainty never causes turn replay.
- **Schema 13** added exact semantic transition clocks to local sessions and
  the then-current remote status projection. The remote projection is removed
  by schema 22.
- **Schema 12** added stable MCP actor aliases.
- **Schema 11** closed a one-time delivery-state gap for sessions migrated from
  the pre-lifecycle daemon.
- **Schema 10** introduced explicit runtime, presentation, work, recovery,
  lifecycle-epoch, attachment-epoch, idle-deadline, and terminal-stop state.

The schema-9-to-10 migration replaced the old `alive`/`working` booleans and
session-claim cleanup model with typed lifecycle and standing tables. It
preserved admission fields, kept ACP and app-server locator kinds distinct, and
initialized local standing from the evidence available in that source schema.

The schema-8-to-9 migration renamed `harness` to `observed_harness`, left
`claimed_harness` and `admitted_bundle` empty, and marked migrated rows with
`endpoint_provenance = 'migration'`. It inferred transport only from exact
locators and remained explicit about facts the old schema could not preserve.

The older schema-7 ownership handoff used an fsynced sidecar so accepted local
write obligations could move into NMP's durable queue before superseded tables
were dropped. That was a migration mechanism, not a current alternate write
path.

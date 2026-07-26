# State schema and upgrades

`state.db` is daemon-owned local persistence. Installing a newer binary must
not create an operator-visible database step: after taking the daemon startup
lock and before serving requests, the store runs every stamped migration from
the installed schema to the binary's current schema.

Each migration is a one-way SQLite transaction. It preserves non-rebuildable
local state, removes the superseded schema in the same commit, and never leaves
a runtime dual-read, legacy key, or compatibility path behind. Rebuildable
`relay_*` projections may be recreated from relay truth; local session,
delivery, signing, and workspace state must be copied intentionally.

The migration table is compile-time-sized from `SCHEMA_VERSION`. Bumping the
version without adding the next contiguous migration does not compile. Tests
start from production-shaped deployed schemas and verify preservation through
the complete chain to current. A malformed source schema fails before its
version or tables are changed.

## Current schema: 17

Schema 17 moves host capability and workspace discovery into the management
key's complete kind:0 profile snapshot. `relay_profiles.agents_json` stores the
host-owned `(slug, about)` inventory and `workspaces_json` stores the roots that
host knows. Updating a profile replaces both sets atomically. The separate
agent-advertisement table is dropped, and cached backend profiles are cleared
during migration so the daemon refetches complete snapshots from relay truth.

Schema 16 removes the transcript path and explicit-chat publication marker from
sessions after transcript auto-publication was removed.

Schema 15 adds `sessions.busy_seconds`, the persisted cumulative working time
used by session lifecycle projections.

Schema 14 adds `native_turn_attempts`, the daemon-owned operational ledger for
RPC turn attempts. Each row correlates the public session, runtime generation,
delivery kind, triggering inbox event when one exists, native thread, native
turn, and one terminal outcome. It does not store presence. Open attempts are
reconciled to `unknown_reconciled` after daemon restart; finished rows expire
after seven days.

Schema 13 adds an exact semantic transition clock to local sessions and remote
status rows. Lease renewal does not advance this clock.

Schema 12 adds stable MCP actor aliases.

Schema 11 keeps schema 10's lifecycle shape and closes the one-time delivery
state gap for sessions migrated from the pre-lifecycle daemon. Historical
`injected` inbox rows are converted to `echo_consumed` only when their target
session is already idle. Pending delivery and injected work owned by a Working
session remain fenced.

Schema 10 keeps schema 9's admitted-runtime facts and makes managed lifecycle
ownership explicit in each `sessions` row:

| Field | Meaning |
|-------|---------|
| `observed_harness` | Harness established from the admitted launch plan or an observed external process. Hook claims never write this field. |
| `claimed_harness` | Last harness name claimed by a hook, retained only for mismatch diagnostics. |
| `admitted_bundle` | Exact configured bundle selected for a hosted launch; empty for externally discovered or migrated sessions. |
| `admitted_transport` | Hosted transport admitted for this runtime: `pty`, `acp`, `app-server`, or empty when unknown/not hosted. |
| `endpoint_provenance` | Source of the endpoint facts: `launch`, `hook`, `migration`, or empty when unavailable. |
| `runtime_state` | Current incarnation state: `running`, `stopping`, or `stopped`. |
| `presentation_state` | PTY presentation: `headed`, `headless`, or `unavailable`. |
| `work_state` | Whether the runtime is `idle` or `working`. |
| `recovery_state` | Exact-session recovery authority: `pending`, `ready`, or irreversibly `revoked`. |
| `lifecycle_epoch` / `attachment_epoch` | Independent fences for runtime transitions and PTY client edges. |
| `idle_since` / `idle_deadline` | Durable headless-idle eviction clock. |
| `stopped_at` / `stop_reason` | Durable terminal transition and its typed cause. |
| `turn_count` | Whether this identity has ever owned a provider turn, independent of a native resume locator. |

Launch admission facts are immutable for a runtime generation. A later hook may
update `claimed_harness`, but cannot reclassify a launch-owned
`observed_harness`, `admitted_bundle`, `admitted_transport`, or
`endpoint_provenance`. Delivery resolves the exact locator keyed by the stored
observed harness and admitted transport; ACP and app-server keep distinct,
generation-fenced locator kinds even though they share the JSON-RPC engine. It never re-reads
mutable agent or bundle configuration to rediscover a live runtime. A newly admitted runtime
generation records its own fresh launch facts.

`session_channels` stores durable channel affinity and recovery authority;
`session_standing` separately stores whether the exact pubkey is a member or
absent. Stopping a runtime does not change either record or remove relay
membership. A confirmed relay admission is committed with the runtime
generation and lifecycle epoch that requested it; stale or failed commits first
persist immediately-due cleanup work so removal can be retried after a daemon
or relay failure. Explicit leave or recovery revocation removes the route and
changes standing to absent only after the corresponding relay cleanup.

Runtime endpoint locators carry their owning generation. PTY supervisor
attachment epochs and exit reports fence late callbacks, while persisted idle
deadlines let restart reconciliation continue the same ten-minute headless-idle
policy. Only explicit forget/revoke changes recovery to `revoked` and removes
the local signer, routes, and locators after process termination is confirmed.

The `inbox` is direct-recipient state, not ambient history. A relay-accepted
event explicitly p-tagging a daemon-owned pubkey creates one pending inbox row
and one recipient edge atomically. Claiming that row does not consult
`session_channels`; it remains claimable when it predates registration or when
the route is later removed. Ambient context reads continue to apply both the
arrival-sequence and signed-time join fences.

Completed offline-mention claims are compact durable tombstones keyed by event
and exact recipient. Unlike ordinary completed operation ledgers, they do not
expire: relay observations can replay old Nostr events, and recovery or process
launch must remain idempotent independently of local chat-cache retention.

The schema-9-to-10 migration replaces the old `alive`/`working` booleans and
session-claim cleanup model with the typed lifecycle and standing tables. It
preserves every schema 9 admission field, keeps ACP and app-server locator
kinds distinct, and initializes standing only from locally recorded routes plus
confirmed relay membership.

The earlier schema-8-to-9 migration renames `harness` to `observed_harness`, leaves
`claimed_harness` and `admitted_bundle` empty, and marks every migrated row with
`endpoint_provenance = 'migration'`. It infers `admitted_transport` only from an
exact `(pubkey, observed_harness)` locator. Codex `acp` locators from schema 8
become `app_server`; other `acp` locators remain ACP. A PTY locator is used only
when neither RPC locator exists. Migrated provenance is deliberately honest
about the facts the old schema could not preserve.

Cross-store ownership changes require a crash-safe handoff. Schema 7 writes
retryable SQLite outbox rows to an fsynced sidecar before dropping the old
tables. The new daemon imports exact group events into NMP's durable queue and
kind:0 events through the profile publisher, deleting each journal only after
acceptance. A crash or unavailable relay therefore causes an idempotent retry,
not a lost write or a blocked schema upgrade.

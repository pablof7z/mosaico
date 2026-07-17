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

## Current schema: 9

Schema 9 makes runtime admission explicit in each `sessions` row:

| Field | Meaning |
|-------|---------|
| `observed_harness` | Harness established from the admitted launch plan or an observed external process. Hook claims never write this field. |
| `claimed_harness` | Last harness name claimed by a hook, retained only for mismatch diagnostics. |
| `admitted_bundle` | Exact configured bundle selected for a hosted launch; empty for externally discovered or migrated sessions. |
| `admitted_transport` | Hosted transport admitted for this runtime: `pty`, `acp`, `app-server`, or empty when unknown/not hosted. |
| `endpoint_provenance` | Source of the endpoint facts: `launch`, `hook`, `migration`, or empty when unavailable. |

Launch admission facts are immutable for a runtime generation. A later hook may
update `claimed_harness`, but cannot reclassify a launch-owned
`observed_harness`, `admitted_bundle`, `admitted_transport`, or
`endpoint_provenance`. Delivery resolves the exact locator keyed by the stored
observed harness and admitted transport; ACP and app-server keep distinct
locator kinds even though they share the JSON-RPC engine. It never re-reads
mutable agent or bundle configuration to rediscover a live runtime. A newly admitted runtime
generation records its own fresh launch facts.

The schema-8-to-9 migration renames `harness` to `observed_harness`, leaves
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

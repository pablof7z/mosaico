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

Cross-store ownership changes require a crash-safe handoff. Schema 7 writes
retryable SQLite outbox rows to an fsynced sidecar before dropping the old
tables. The new daemon imports exact group events into NMP's durable queue and
kind:0 events through the profile publisher, deleting each journal only after
acceptance. A crash or unavailable relay therefore causes an idempotent retry,
not a lost write or a blocked schema upgrade.

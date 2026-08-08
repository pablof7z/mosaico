# NMP subscription stress harness

This hermetic binary compares the boundaries implicated in the Mosaico CPU
incident without opening a relay connection or reading `~/.mosaico`.

Run the captured-shape comparison:

```sh
just stress-nmp
```

Defaults model 207 standing semantic watches: 180 independent kind:9 `#p`
mailboxes plus 27 other tagged watches. They are compared with set-valued
shards, 64 short-lived windowed profile lookups, an empty in-memory NMP engine,
and a disposable populated redb store. The fixed seed is `29`.

Useful focused runs:

```sh
just stress-nmp --scenario router --iterations 20
just stress-nmp --scenario consumer --topology per-identity
just stress-nmp --scenario store --corpus-rows 10000 --format csv
just stress-nmp --help
```

The output labels three authority boundaries:

- `public_facade` measures the supported `Engine::observe`/cancel/shutdown
  lifecycle. It cannot see NMP's private store-query count or split router
  phases.
- `internal_control` uses NMP's opt-in headless reducer, store work counters,
  resolver, router, coalescer, and plan-diff seams. These are attribution
  controls, not an app API or product telemetry contract. The headless reducer
  reports exact Redb event-projection rows, coverage-table point reads,
  lifecycle transitions, and wire/row/evidence/diagnostics effects.
- `mosaico_consumer` adds the current Mosaico shape of one blocking drain OS
  thread per retained observation, then joins every thread during teardown.

All public-facade queries use `Freshness::CacheOnly`. The headless reducer uses
`Freshness::Live` but only records its effects; no runtime executes them. The
only relay URL is under the reserved `.invalid` domain and is never dialed. The
redb file lives in a `tempfile` directory removed on exit. No secrets, live
daemon, fixed port, or external executable are used.

## Reading the attribution

`core_live_incremental_open` is the synchronous app-facing work: each new
observation gets its own cached projection while relay execution remains
pending. `core_pending_admission` completes one pending cohort. Its expected
signature is one router compile, no event projection, and no rewrite of a
running REQ. `core_live_incremental_close` measures immutable withdrawal.
`core_runtime_presubscribe_tick` isolates the durable-maintenance pass that
the public runtime currently performs immediately before every subscribe.
It intentionally has no query, router, or wire effects; non-zero time there
is unrelated subscription overhead tracked in NMP issue #1344.

The router controls separate the product admission path from counterfactual
work:

- `router_pending_cohort_admit` routes and groups one unsent cohort.
- `router_active_attach` asks whether already-running exact coverage makes a
  new request unnecessary.
- `router_incremental_full_recompile` is the superseded control that compiles
  the whole growing demand set after every arrival.
- `selection_coalesce`, `wire_plan_diff_stable`, and
  `router_stable_recompile` isolate grouping, plan comparison, and whole-plan
  routing respectively.

For the default fixture, the pre-fix reducer opened 207 observations with 207
router compiles and 241,018 Redb index/event rows, then closed them with
512,462 more rows. Pending admission reduces that to 207 independent cache
seeds reading 3,640 rows, followed by one compile reading zero event rows.
Closing reads zero event rows. Wall-clock timings vary by machine; the exact
work counters are the primary regression oracle.

On repeated captured-shape runs, those 207 no-op pre-subscribe ticks took
836-875 ms wall time in aggregate while the corresponding headless opens took
60-63 ms. Both reported zero event/index rows, zero coverage reads,
zero router compiles, and zero effects. That explains most of the remaining
public-facade Redb wall-time gap as durable-maintenance latency, not relay
grouping or cached event projection.

The runtime-owned 30 ms deadline is tested in NMP against an in-process relay.
This harness drives the corresponding reducer flush explicitly so scheduling
jitter cannot contaminate attribution. It also compares 207 consumer
observations with four set-valued shards: NMP's pending admission fixes
cross-observation recomputation, while sharding independently measures the
remaining Mosaico consumer-cardinality cost.

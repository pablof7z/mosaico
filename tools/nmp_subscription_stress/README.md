# NMP subscription stress harness

This hermetic binary compares the boundaries implicated in the Mosaico CPU
incident without opening a relay connection or reading `~/.mosaico`.

Run the captured-shape comparison:

```sh
just stress-nmp
```

Defaults model 207 standing semantic watches: 180 independent kind:9 `#p`
mailboxes plus 27 other tagged watches. A set-valued sharded topology remains
only as a counterfactual attribution control; it is not an application fix.
Applications may own thousands of independent observations and NMP must scale
without requiring them to aggregate. The harness also includes 64 short-lived
windowed profile lookups, an empty in-memory NMP engine, and a disposable
populated redb store. The fixed seed is `29`.

Useful focused runs:

```sh
just stress-nmp --scenario router --iterations 20
just stress-nmp --scenario consumer --topology per-identity
just stress-nmp --scenario store --corpus-rows 10000 --format csv
just stress-nmp --scenario freshness --retained 207 --corpus-rows 2000
just stress-nmp-matrix --matrix-counts 1,32,207,1000 --format csv
just stress-nmp-matrix --matrix-counts 10000 --demand-shape exact-duplicate --lifecycle-schedule reverse
just stress-nmp-daemon
just stress-nmp --help
```

`stress-nmp-daemon` is the final product boundary: it starts the real Mosaico
daemon with a disposable home/database and counted local relay, opens exactly
207 profile observations through Mosaico's startup policy, records CPU, RSS,
threads, file descriptors, RPC latency, REQ/CLOSE counts, and proves exact
process/socket/relay teardown. It never touches the selected live instance.

Every result carries the full NMP commit resolved in `Cargo.lock`. Candidate
comparisons use isolated Mosaico worktrees whose `Cargo.toml` and `Cargo.lock`
pin the exact pushed NMP commit. Path-patched or otherwise unattributed builds
refuse to run. There is no free-text revision label, so baseline and candidate
results cannot be mixed under a fabricated name.

Each row also carries an explicit status: ordinary measurements are
`measured`, enforced semantic rows are `contract_pass`, unmet target behavior
is `known_red`, the current complete full-B residual fallback is
`known_red_safe_full_b`, and a row with no honest public driver is
`unavailable`.

## Adversarial lifecycle matrix

The matrix keeps every application observation independent. It varies:

- cardinality: 1, 32, 207, 1,000, 4,096, and 10,000;
- exact duplicate, compatible-distinct tag demand, no-limit kind:0 author
  demand matching the avatar workload, limit:1 incompatible demand, and
  unlimited demand incompatible across tag and `since` axes;
- forward, reverse, seeded-random, pre-admission, and interleaved cancellation;
- a duplicate arriving after exact physical coverage is already running; and
- a second compatible cohort arriving after the first request was sent;
- every non-final duplicate-owner close versus the final physical close;
- detach and re-attach while a sibling keeps the immutable REQ alive; and
- 1%, 50%, and all-but-one pending cancellations before survivor admission;
- two disjoint profile-churn waves while one standing owner proves partial
  attribution cleanup does not wait for total teardown;
- the same `CoverageKey` under distinct unbounded and `limit:1` windows,
  proving separate immutable REQs and observation-scoped request evidence;
- same-filter Live and CacheOnly observations in both close orders, proving
  only Live owns execution evidence and wire lifetime;
- scalable mixed Live, CacheOnly, current/stale/missing MaxAge cohorts; and
- nested same-DemandKey occurrences with a Live inner demand and either a
  CacheOnly or satisfied-MaxAge outer occurrence;
- a later exact owner attaching to an accepted immutable REQ, followed by
  either owner withdrawing before the incumbent EOSE settles only the survivor.
- two compatible observations admitted in the same pending cohort grouping
  into one REQ;
- a merely compatible later observation executing a second immutable REQ;
- a later demand fully covered by an active semantic superset attaching to it;
- a partially covered later demand executing only its exact uncovered
  residual; and
- the surviving later owner retaining both incumbent and residual physical
  coverage after the incumbent app observation closes.

The exact later-owner case covers attachment while the incumbent request is
still outstanding. It deliberately does not model historical replay for an
owner that attaches after that request has already settled. Semantic-superset
attachment and exact residual subtraction are reported as `known_red` until
NMP implements them; their expected filters and ownership consequences remain
explicit rather than being weakened to today's full-filter second REQ.

Each open must return a unique observation identity and its own local frame.
The exact-covered core attach case requires zero router compiles and zero new
wire work. A direct-router re-admission of the same `DemandKey` is labeled as
a no-op, not as a second observation owner. The two-wave case requires later
uncovered work to add a REQ without closing or rewriting the request already
sent. Every teardown requires an exact zero
census across observation, history, resolver, request-target indexes, pending,
wire, attribution, router, and execution ownership. Exact Redb work, coverage
reads, router compiles, request-target candidates, incumbent router-index
visits, REQs, CLOSEs, frames, CPU, wall time, file descriptors, peak RSS, and
surviving NMP threads are reported separately. Process-global resource counts
are diagnostic; exact NMP-owned counters are the pass/fail oracle. The current
NMP seam does not expose router/coalescer candidate-pair counts, so the harness
does not invent a proxy assertion for them.

The matrix caps internal/public loads at 10,000. The consumer-thread boundary
is capped at 1,024 because deliberately creating one OS drain thread per
observation is itself the resource hazard being measured. All random order is
seeded and replayable. A failed matrix run retains a sanitized record under
`target/nmp-stress-failures/` with the exact full-argument replay command,
NMP/harness revisions, lock hash, and an opaque failure hash. The raw error is
deliberately excluded so host paths or secrets cannot enter the artifact.
Successful runs create no artifact and continue to remove every disposable
store on drop.

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

The public-facade phase measures both `Freshness::CacheOnly` and
`Freshness::Live`; the Mosaico-style consumer phase is deliberately Live.
Headless controls use explicit Live, CacheOnly, and MaxAge policies and only
record reducer effects. Synthetic accepted-handoff edges are fed directly to
the reducer to inspect observation-scoped `RelayRequest` evidence. The only
configured relay URL uses the reserved `.invalid` domain, so a Live runtime
cannot reach a real relay. The redb file lives in a `tempfile` directory
removed on exit. No secrets, live daemon, fixed port, or external executable
are used.

The explicit `freshness` scenario compares identical headless kind:0 profile
observations under `Live` and missing-coverage `MaxAge`. The matrix adds
current and stale coverage variants. Both leave relay admission pending, so
the measured difference is the opening-time freshness/plan decision rather
than socket work or the admission timer.

The `profiles` scenario keeps the existing short-lived CacheOnly lookup
control and separately opens an unlimited Live kind:0 observation for every
profile. The Live cohort remains open for at least 25 ms, spanning the 10 ms
first-arrival grouping window, before every independent observation is
cancelled. This is the public-facade version of the avatar burst that motivated
the grouping contract; it never adds a `limit` to the replaceable-event query.

The harness does not yet claim deterministic store-read failure, a command
ordered at the same instant as a due admission deadline, old-versus-future
replaceable-document freshness, or reuse of NMP-written durable coverage after
an engine restart. Reopening the seeded Redb fixture and explicitly flushing
the headless reducer are not substitutes for those controls.

## Reading the attribution

`core_live_incremental_open` is the synchronous app-facing work: each new
observation gets its own cached projection while relay execution remains
pending. `core_pending_admission` completes one pending cohort. Its expected
signature is one router compile, no event projection, and no rewrite of a
running REQ. `core_live_incremental_close` measures immutable withdrawal.
`core_counterfactual_tick_sweep` isolates the durable-maintenance pass that
the pre-#1344 public runtime performed immediately before every subscribe.
It intentionally has no query, router, or wire effects. A #1344 candidate is
validated by NMP's public-runtime test proving ordinary opens execute zero
maintenance sweeps; this control remains only to quantify the removed work.

The router controls separate the product admission path from counterfactual
work:

- `router_pending_cohort_admit` routes and groups one unsent cohort.
- `router_existing_demand_readmit` is the pure-router no-op for a `DemandKey`
  already present; the core scenarios own real observation attachment.
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

The runtime-owned 10 ms deadline is tested in NMP against an in-process relay.
This harness drives the corresponding reducer flush explicitly so scheduling
jitter cannot contaminate attribution. It also compares 207 consumer
observations with four set-valued shards solely to isolate the remaining
Mosaico consumer-cardinality cost. Sharding is not prescribed to callers.

When `--matrix-counts` includes 10,000, the load matrix also measures
candidate-only metadata work over one request carrying 10,000 historical
claims, and whole-plan metadata reconciliation with one survivor out of 10,000
prior requests. Smaller bounded check matrices skip those two deliberately
heavy controls. The post-EOSE retry load is emitted as `unavailable`: NMP's
public benchmark door exposes its retry counters but does not expose the
coverage-write fault injector or metadata-transfer driver needed for an honest
external replay. The harness does not reproduce private NMP state or report a
fabricated measurement.

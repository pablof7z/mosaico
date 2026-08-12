# BDD and the Cucumber behavior-contract suite

Mosaico uses BDD as a discovery and specification discipline. Concrete
examples, counterexamples, public observables, and must-never consequences are
chosen before implementation.

Cucumber is not the foundation or complete product catalog. The custom target
under `bdd/` executes only a small set of critical, deterministic,
cross-boundary fabric contracts from `features/`. Most executable behavior
belongs in ordinary Rust tests.

The canonical agent guidance starts at
[`skills/mosaico-dev/resources/testing/INDEX.md`](../../skills/mosaico-dev/resources/testing/INDEX.md).

## Admission

A `.feature` scenario must be:

- a stable operator/agent-visible promise;
- deterministic with controlled local fixtures;
- vulnerable to passing lower-level tests while the product remains broken;
- load-bearing enough to justify step-glue maintenance;
- clearer as a concrete product-language example;
- observable through a public or independent oracle.

Adapter matrices, parser cases, future plans, known failures, timing
permutations, emergent model behavior, live-provider checks, and removed
feature tombstones are not admitted.

Every committed scenario executes. The suite has no `@designed`, `@wip`,
`@live`, historical migration, or issue tag semantics. GitHub owns plans,
seeded fault tests own schedules, evaluation datasets own model-dependent
capabilities, and `mosaico-dev` live labs own real-provider compatibility.

## Run

The suite requires `nak` on `PATH` and an external Croissant executable:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-behavior-contracts
```

CI checks out Croissant commit
`9c4c93e84852bd9aa6824060b74c56ab2ce812c2`, builds it with Go 1.25.12, pins
`nak` 0.20.0, and runs the same recipe.

## Runner

`Cargo.toml` declares `bdd/main.rs` as a custom integration target with
`harness = false`. The target receives `CARGO_BIN_EXE_mosaico`; scenarios never
search `PATH`, invoke nested Cargo, or fall back to an installed Mosaico.

Each scenario receives a fresh `MosaicoWorld` containing:

- isolated backend homes, configs, workspaces, sockets, and identities;
- scenario-owned `nak` or external Croissant;
- deterministic native harness shims;
- exact child-process handles and bounded cleanup;
- public command results and independent relay/harness witnesses.

Scenarios are serialized. Eventual assertions poll observable evidence to a
deadline. Failed worlds are copied to
`target/bdd-artifacts/<scenario>/` before teardown.

## Observables

The suite may inspect:

- CLI and hook exit status, stdout, and stderr;
- relay events queried independently with `nak`;
- daemon sockets and exact child-process liveness;
- native harness delivery captures;
- stable public identities.

It does not inspect SQLite. Exact adapter argv and protocol matrices belong in
typed Rust conformance/process tests.

Delivery counts are scoped to those observables. “One harness-visible delivery
during the controlled execution” means the deterministic capture remains at
one through a bounded observation window. It is not a claim of global
exactly-once processing across crashes, retries, or all future time.

## Current scope

The current eight scenarios passed admission independently:

| Product contract | Why Cucumber earns its cost | Public or independent witness |
|---|---|---|
| Relay membership warms peer identity without exposing backend authority | Joins the retained NMP group/profile observations, roster rendering, and the management-identity boundary | Rendered roster plus relay-only peer |
| A backend-addressed management request produces a public result | Joins operator-authored relay input, backend routing, command execution, and reply publication | Independently queried relay reply |
| Addressed work produces one harness-visible delivery during controlled execution | Joins operator addressing, relay delivery, daemon routing, PTY injection, and bounded duplicate observation | Deterministic native-harness capture |
| An explicit sender session overrides ambient process hints | Protects signer authority across CLI selection, ambient state, and relay publication | Relay event author |
| Isolated backends discover a workspace only through the relay | Proves cross-backend convergence without shared homes, stores, sockets, or workspaces | Second backend's public channel listing plus filesystem isolation |
| Lifecycle hooks fail open without starting backend infrastructure | Protects a load-bearing authority and latency boundary at the exact hook entry point | Hook result, elapsed time, and absent daemon socket |
| Addressed work resumes a stopped session without minting a sibling identity | Joins offline delivery, native resume, stable public identity, and user-visible injection | Public session identity plus harness capture |
| Addressed work starts an offline stable agent under its configured identity | Joins configured durable identity, offline activation, relay delivery, and user-visible injection | Public session identity plus harness capture |

Current-observation search does not by itself earn a Cucumber scenario. Query
filtering, grouping, pagination, and rendering belong in typed Rust and
real-binary integration tests. A behavior-contract scenario is warranted only
when it exercises the cross-boundary ownership promise through public evidence,
such as a controlled NMP removal making relay state disappear or publish
acceptance producing no local delivery before observation. Such a scenario
must control the observation boundary and use CLI, relay, or harness output as
its oracle; it must never inspect SQLite to prove the absence of replication.

When a scenario stops earning admission, move any still-current claim to its
proper Rust, fault, evaluation, or live layer and delete its exclusive glue.
Every scenario must earn that admission independently; appearing in this scope
list does not admit it automatically.

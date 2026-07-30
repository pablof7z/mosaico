# BDD and deterministic behavior contracts

BDD is Mosaico's discipline for discovering and specifying current behavior
through examples. Cucumber is a small executable surface for critical,
deterministic, cross-boundary product contracts.

## What Cucumber owns

The admitted suite proves load-bearing fabric behavior where lower-level tests
can pass while Mosaico remains broken:

- relay-only identity and discovery across isolated backends;
- addressed delivery through the real binary, daemon, relay, and harness;
- public-key continuity without sibling identities;
- explicit sender authority;
- management replies visible through the relay;
- hook fail-open and authority boundaries.

It does not catalog every product domain, command, adapter, bug, or future
feature.

## What belongs elsewhere

- Local rules and input spaces: unit/property tests.
- Harness, relay, PTY, ACP, and provider equivalence: typed adapter suites.
- Exact selector and argv matrices: adapter/process contracts.
- Races and restart permutations: seeded fault/schedule tests.
- Emergent awareness and coordination: repeated agent evaluations.
- Real providers and public relays: `mosaico-dev` live labs and probes.
- Future behavior and known gaps: GitHub Issues.

## Current runner

`Cargo.toml` declares `bdd/main.rs` as a custom integration target. It parses
the narrowly admitted `features/` tree and uses `CARGO_BIN_EXE_mosaico`.

Each `MosaicoWorld` owns:

- a temporary root;
- isolated homes, configs, workspaces, sockets, and identities;
- scenario-owned `nak` or external Croissant;
- deterministic native harness shims;
- child-process handles and bounded cleanup;
- public command results and independent relay/harness witnesses.

Scenarios are serialized. Eventual assertions poll evidence to a deadline.
Failed worlds are retained under `target/bdd-artifacts/<scenario>/`.

Allowed observables include CLI/hook results, relay events queried
independently, exact process state, harness delivery captures, and public
identity. SQLite is not a Cucumber observable.

## No catalog tags

Committed feature files contain only executable deterministic contracts.

- `@croissant` documents a required local NIP-29 fixture.
- `@must-never` identifies a deterministic safety contract.

There is no `@designed`, `@wip`, `@live`, historical migration, or issue tag
semantics. A skipped plan is not executable evidence. Keep planned behavior in
its GitHub issue, develop failing scenarios on the implementation branch, and
commit them only with their implementation.

## Oracle handoff

The design agent:

1. discovers examples and counterexamples;
2. proves the claim satisfies the Cucumber admission rule;
3. identifies public and independent oracles;
4. authors the failing scenario and minimal vocabulary.

The implementation agent may add lower-level evidence but cannot silently
change the outcome. An adversarial pass tries false-pass implementations,
contrast cases, and boundary failures before completion.

## Running

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-behavior-contracts
```

The suite is deterministic and credential-free. Provider/network checks use
the separate live-lab and probe commands.

## Retirement

If a contract no longer meets admission, move its still-current claim to the
appropriate Rust/fault/evaluation layer and remove its Gherkin glue.

If the product behavior itself is removed, delete the scenario and exclusive
steps. Never replace it with a scenario proving the dead surface is absent.

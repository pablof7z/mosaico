# Behavior-driven development in Mosaico

BDD is the practice of discovering product behavior through concrete examples,
expressing those examples in shared language, and making the resulting claims
executable.

Gherkin is the notation Mosaico uses to preserve the result. Cucumber is the
runner. Neither tool creates BDD without the example-discovery work.

## What BDD owns

Mosaico's feature tree owns stable behavior visible to operators and agents:

- setup and diagnostics;
- agent discovery and selection;
- workspace and channel behavior;
- session identity and lifecycle;
- awareness and messaging;
- coordination and management;
- native harness launch behavior;
- daemon ownership and recovery;
- multi-backend fabric behavior;
- must-never authority, durability, and secrecy boundaries.

The feature files are not a catalog of Rust modules. Parsers, migrations,
storage mechanics, codecs, and narrow races remain lower-level evidence.

## The BDD conversation

Before writing a scenario, the design agent should answer:

1. Who observes the behavior?
2. What useful state already exists?
3. What event or decision occurs?
4. What outcome is externally meaningful?
5. What failure would violate trust?
6. Which independent witness can see it?
7. Which examples distinguish the rule from nearby interpretations?

For a stable agent, useful examples include an offline configured identity,
addressed work, activation under that exact public key, one harness delivery,
and no sibling identity. They do not include the internal method used to choose
the signer.

## Current harness contract

`Cargo.toml` declares a custom integration target at `bdd/main.rs` with
`harness = false`. The runner parses `features/` and uses
`CARGO_BIN_EXE_mosaico`, so scenarios drive Cargo's exact binary rather than an
installed executable.

Each `MosaicoWorld` owns:

- a temporary root;
- isolated backend homes, configs, workspaces, sockets, and identities;
- a scenario-owned `nak` or external Croissant relay;
- deterministic native harness shims;
- child-process handles and bounded cleanup;
- public command results and independent relay/harness witnesses.

Scenarios are serialized. Waits poll evidence until a deadline. Failed worlds
are retained under `target/bdd-artifacts/<scenario>/`.

BDD may observe CLI/hook status and output, relay events queried independently,
socket/process state, harness argv or delivered-input captures, and stable
diagnostic JSON. It must not inspect SQLite tables.

## Tags are truth claims

- Untagged scenarios are built, deterministic, and run by default.
- `@must-never` is deterministic and still runs by default.
- `@croissant` states a fixture requirement; it does not skip CI.
- `@live` requires real provider auth or public infrastructure and is opt-in.
- `@designed @issue-N` records agreed behavior that is not implemented.
- `@wip @issue-N` records a built behavior with a known failing contract.
- `@bdd-N` preserves historical migration traceability only.

`@designed` and `@wip` do not count as passing coverage. Their issue must be
open and behavior-specific. When that issue closes, remove the exclusion and
run the scenario, move it to another valid open owner, or correct/remove the
claim.

Several current designed scenarios use umbrella harness issue `#704`. Treat
that as a known ownership gap. Do not copy that pattern into new scenarios.

## Feature removal

When Mosaico intentionally removes a feature, delete its feature scenarios and
exclusive step vocabulary. Do not add a scenario asserting that the former
feature, command, or option is absent. The feature tree describes the product
that exists now, not the history of surfaces it has deleted.

## Test-driven handoff

The design/architecture agent normally:

1. writes or revises the feature example;
2. identifies the public witness;
3. adds only the step vocabulary needed for the claim;
4. confirms the scenario fails for the intended missing behavior.

A separate implementation agent then changes production code and supporting
lower-level tests. It must not change expected behavior merely to make the
scenario pass. If the claim is wrong, return it to design with concrete
evidence.

## BDD anti-patterns

- Scenario-per-function or scenario-per-command-option.
- Steps that invoke named Rust tests.
- Database assertions presented as product outcomes.
- Mock-only worlds that bypass the exact binary.
- Vague outcomes such as “works correctly.”
- Fixed sleeps as acceptance semantics.
- A second prose or shell acceptance catalog.
- Real model-quality assertions in deterministic acceptance.
- Exclusion tags without a specific open owner.

Run deterministic BDD with:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-bdd
```

Run the opt-in live tier with:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-bdd-live
```

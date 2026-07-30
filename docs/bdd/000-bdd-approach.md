# Executable BDD approach

Mosaico's acceptance contract is the Gherkin tree under `features/`. The
custom Cargo target in `bdd/` parses that tree and drives the exact Mosaico
binary Cargo just built.

This layer owns stable product behavior. Unit and integration tests continue
to own parsers, codecs, migrations, state machines, storage mechanics, and
narrow races.

## Run the suite

The deterministic suite requires `nak` on `PATH` and an external Croissant
executable:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-bdd
```

The opt-in provider tier is:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-bdd-live
```

CI checks out Croissant commit
`9c4c93e84852bd9aa6824060b74c56ab2ce812c2`, builds it with Go 1.25.12, pins
`nak` 0.20.0, and runs the same `just test-bdd` recipe.

## Architecture

`Cargo.toml` declares a root-package integration target with
`harness = false`. Keeping the target in the package that owns the binary gives
the runner `CARGO_BIN_EXE_mosaico`; scenarios never search `PATH`, invoke nested
Cargo, or fall back to an installed Mosaico.

Each scenario receives a fresh `MosaicoWorld` containing:

- one temporary root;
- isolated backend homes, configs, workspaces, sockets, and identities;
- a scenario-owned `nak` or Croissant relay;
- deterministic PTY/harness shims;
- exact child-process handles and bounded cleanup;
- the most recent public command result and external witness state.

Scenarios are serialized. Every asynchronous assertion polls observable
evidence until a deadline. Fixed sleeps are not scenario semantics.

Successful worlds are removed. A failed world's logs, captures, configs, and
relay data are copied to `target/bdd-artifacts/<scenario>/` before teardown.

## Observables

Acceptance steps may use:

- CLI and hook exit status, stdout, and stderr;
- relay events queried independently with `nak`;
- daemon sockets and exact child-process liveness;
- native harness argv, identity, and delivered-input captures;
- stable diagnostic JSON.

BDD steps do not inspect SQLite tables. A behavior without a supported public
witness belongs in a lower-level test until the product exposes one.

## Tags

- Untagged scenarios are built, deterministic behavior and run by default.
- `@must-never` marks a deterministic safety invariant and still runs by
  default.
- `@croissant` documents that the scenario requires the real local NIP-29
  fixture; it does not skip CI.
- `@live` requires real provider authentication or public infrastructure and
  runs only in the live tier.
- `@designed @issue-N` is agreed acceptance for behavior that is not built.
- `@wip @issue-N` is built behavior with a known failing contract.

The runner always excludes `@designed` and `@wip`, and rejects either tag
without an issue tag. A skipped scenario must never look like passing evidence.

Numbered `@bdd-N` tags preserve traceability for contracts migrated from the
former prose matrix. The tag is historical metadata; the feature sentence is
the canonical description.

## Authoring discipline

Feature prose names people, operators, agents, sessions, workspaces, channels,
messages, visible outcomes, and failures. Rust modules, tables, and internal
RPC names stay out of scenarios unless that protocol surface is itself the
contract.

A new built scenario must:

1. use the exact binary through the existing world;
2. create all state in its own scenario root;
3. assert at least one public or independent witness;
4. bound every wait;
5. leave no child process or socket behind;
6. run without credentials unless tagged `@live`.

Do not add a second shell or prose acceptance catalog. Extend the feature tree,
closed step vocabulary, and world fixtures instead.

## Coverage shape

The feature tree covers setup, agent discovery, workspace scope, channels,
sessions, awareness, messaging, coordination, native harnesses, daemon
ownership, diagnostics, multi-backend behavior, and safety boundaries.

Built deterministic scenarios include the migrated relay and launch contracts:

- relay-only workspace discovery across isolated backends;
- exact native profile argv;
- addressed PTY delivery;
- stopped-session recovery without a sibling identity;
- stable-agent activation under its configured key;
- explicit sender-session precedence;
- backend-addressed management routing;
- portable PTY launch without the retired terminal host;
- relay-only profile warming;
- backend management-key roster exclusion.

Issue-linked designed scenarios make remaining product gaps visible without
claiming they pass.

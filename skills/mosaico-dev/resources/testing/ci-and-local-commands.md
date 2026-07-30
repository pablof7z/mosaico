# CI and local commands

Commands are evidence entry points. Report exactly what ran and which external
dependencies were supplied.

## Aggregate local suite

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test
```

`just test` aliases `test-all-local`, which currently runs:

- `test-dev-scripts`;
- `test-site`;
- `test-unit`;
- `test-hermetic-integration`;
- `test-local-relay`;
- `test-local-nip29`;
- `test-behavior-contracts`.

It requires Rust, Node, `nak`, and an external Croissant executable. It does not
run credentialed live checks.

## Deterministic suites

Library tests:

```sh
just test-unit
```

This is `cargo test --lib`.

Hermetic real-binary integration:

```sh
just test-hermetic-integration
```

This executes `tests/help.rs` and `tests/install_standalone.rs` without a relay
or external executable.

Plain local relay:

```sh
just test-local-relay
```

This runs `daemon_mechanics` and `e2e_transport` and requires `nak`.

Local NIP-29 daemon integration:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-local-nip29
```

This runs `daemon_integration` with one Rust test thread.

Executable product contracts:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-behavior-contracts
```

Development scripts and site:

```sh
just test-dev-scripts
just test-site
```

`test-dev-scripts` currently runs `skills/mosaico-dev/tests/scripts.sh` and
`scripts/tests/install-fleet.sh`. `test-site` builds and tests the static site.

## Quality gates

```sh
just fmt-check
just lint
just loc-check
```

`lint` runs Clippy on all targets with warnings denied. `loc-check` also checks
integration-helper imports. These gates constrain source quality; they do not
prove product behavior.

## Current GitHub Actions shape

`.github/workflows/ci.yml` currently has:

- `quality-gate`: formatting, LOC/helper checks, development scripts, site,
  Clippy, library unit tests, and hermetic real-binary integration;
- `local-relay-integration`: pinned `nak` and the plain local relay suite;
- `behavior-contracts`: pinned Croissant/Go/`nak` and the narrow deterministic
  Cucumber contract suite.

The local NIP-29 `daemon_integration` recipe is not currently a separate CI
job. Do not report it as CI-covered unless it was explicitly run.

## Live checks

Relay probes:

```sh
MOSAICO_RELAY=wss://intentional-relay just test-live-relay-probe
MOSAICO_NIP29_RELAY=wss://intentional-relay just test-live-nip29-probe
```

Validation seed:

```sh
MOSAICO_NIP29_RELAY=wss://intentional-relay just test-live-seed-validation
```

These may publish public disposable state. Run them only against an explicitly
chosen relay and report that choice.

Real-provider transport/auth checks run through the `mosaico-dev` live-lab
workflow. Agent capability evaluations and long seeded schedule campaigns are
separate evidence families; they do not run through Cucumber.

## Focused development

Use Cargo's target and name filters to shorten the red/green loop. Before
handoff, run the full owning recipe plus adjacent suites that share the changed
authority boundary.

Do not replace execution with `cargo check`, Clippy, compilation, or test
listing. Record skipped suites and missing external dependencies explicitly.

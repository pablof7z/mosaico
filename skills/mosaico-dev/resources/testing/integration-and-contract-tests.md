# Integration and adapter contract tests

Integration tests prove that owned Mosaico components compose correctly.
Contract tests protect an exact technical boundary. “Integration” describes
execution scope; “contract” describes the claim. One test can be both.

Adapter conformance is a first-class Mosaico test family. A shared typed suite
should be applied to every implementation of the same semantic boundary rather
than copying similar examples into provider- or harness-specific tests.

## Integration scope

Use integration evidence for:

- daemon client and Unix-socket behavior;
- store plus domain policy;
- NMP acquisition plus Mosaico materialization;
- session lifecycle plus harness adapter;
- concurrency, restart, and failure recovery;
- CLI/RPC behavior that needs the real binary but is too technical for
  product-readable Gherkin.

Current targets include:

- `tests/daemon_mechanics.rs` for spawn, socket, race, and version mechanics;
- `tests/e2e_transport.rs` for Nostr codec and NMP acquisition boundaries;
- `tests/daemon_integration.rs` for daemon, messaging, channel, signer, and
  harness composition;
- `tests/help.rs` and `tests/install_standalone.rs` for hermetic real-binary
  CLI/install behavior.

## Contract scope

Use contract tests when exact shapes or rejection rules are load-bearing:

- JSON config fields and unknown-field behavior;
- RPC request, response, and protocol-skew frames;
- Nostr event tags and authorship;
- harness-native profile selectors;
- ACP/app-server protocol dialects;
- hook input and output envelopes.

Assert the accepted and rejected shapes that define the current contract.
When a field, alias, protocol dialect, or whole surface is deliberately
removed, delete tests and fixtures dedicated to it. Do not retain its name in a
negative contract test merely to prove the former surface remains absent.

## Shared adapter suites

Use one conformance suite when multiple implementations promise the same
Mosaico semantics:

- PTY harness drivers;
- ACP and app-server dialect adapters;
- native profile selectors;
- relay implementations;
- hosted transport lifecycle;
- provider hook/plugin integrations.

The suite should accept a typed factory or adapter and assert common identity,
delivery, lifecycle, error, and cleanup outcomes. Implementation-specific tests
then cover only genuine differences.

Do not use Cucumber outlines as an adapter matrix. Do not launch real models to
prove deterministic selector, frame, or lifecycle composition.

## Observer rules

Technical tests may inspect the owned store, protocol frames, logs, or process
metadata when that seam is the subject. State exactly why the observer is
authoritative.

Do not present an internal observer as product acceptance. If the claim is “the
operator sees the peer by name,” use the public roster in BDD. A lower-level
profile-materialization test may separately inspect stored profile state.

## Real components and doubles

Prefer real owned Mosaico components. Replace a boundary when:

- the real dependency is outside repository ownership;
- behavior is credentialed, costly, destructive, or variable;
- a deterministic failure is otherwise impossible to produce;
- the test needs to capture exact interaction at the boundary.

Use `nak serve` for plain Nostr behavior and Croissant for NIP-29 group
semantics. Use deterministic harness shims for native executable behavior. Do
not mock away the exact seam the test claims to prove.

## Isolation and concurrency

Integration targets that mutate environment must serialize those mutations.
Every daemon needs an isolated `MOSAICO_HOME`, config, socket, and working
directory. Use ephemeral ports and clean exact child processes.

Concurrency tests should assert the invariant—one daemon/socket/writer—not a
specific thread schedule. Poll for bounded observable state rather than using a
sleep as proof.

## Relationship to behavior contracts

Keep integration tests beneath a BDD scenario when they add:

- faster localization;
- exhaustive boundary cases;
- protocol-shape evidence;
- race or failure-injection detail;
- owned-store invariants that are intentionally not public.

Remove an integration test only when it duplicates the same setup, action,
oracle, and failure value as broader evidence.

## Running

Plain local relay targets:

```sh
just test-local-relay
```

Croissant-backed daemon integration:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-local-nip29
```

Known command gap: no named `just` recipe currently executes
`tests/help.rs` or `tests/install_standalone.rs`. Run them explicitly when
touching those surfaces:

```sh
cargo test --test help
cargo test --test install_standalone
```

Document this as a current gap; do not claim Clippy execution is test
execution.

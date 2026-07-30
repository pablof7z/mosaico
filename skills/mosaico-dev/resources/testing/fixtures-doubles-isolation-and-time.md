# Fixtures, doubles, isolation, and time

Test infrastructure is part of the proof. A correct assertion over leaked or
uncontrolled state is not reliable evidence.

## Vocabulary

- Fixture: controlled input or starting state, such as an agent config,
  workspace map, relay event, or temporary home.
- Fake: a lightweight working implementation of a boundary.
- Stub: returns a controlled response.
- Spy or capture: records an interaction for assertion.
- Shim: an executable or protocol peer that stands in for a native harness.
- Mock: a double whose expected interactions decide pass/fail.

Use the most behavior-oriented double possible. Mosaico commonly needs process
shims and captures because exact argv, prompts, and delivery are the contract.
Avoid mocks that freeze internal call order.

## Fixture rules

Every fixture should:

- contain only state needed by the claim;
- use valid current config and identity shapes;
- make relevant differences visible in the test;
- be created inside an owned temporary root;
- be disposable without touching host state;
- avoid copied production secrets.

When a config field, CLI surface, or feature is deliberately removed, delete
its fixtures along with its tests. Do not keep a rejection fixture whose only
purpose is to prove the dead surface remains absent.

## Isolated homes and environment

Process tests give each backend separate:

- `HOME`;
- `MOSAICO_HOME`;
- `MOSAICO_CONFIG`;
- workspace roots and maps;
- daemon socket, lock, logs, and state;
- harness home and executable captures.

Clear or explicitly set environment variables inherited from the developer
shell. In particular, do not let an exported `MOSAICO_BIN`, provider home, or
host `PATH` select an unintended executable.

When a Rust test must mutate process-global environment, guard and serialize
the mutation. Restore prior values on drop.

## Identities and secrets

Generate deterministic disposable identities only when stable assertions need
them. Preserve the Mosaico distinction between operator signer,
backend-management identity, per-session identities, and durable agent keys.

Assertions and debug output may use public keys or short public prefixes.
Never print `userNsec`, `mosaicoPrivateKey`, agent secret keys, or provider
credentials.

## Ports, processes, and cleanup

Bind loopback port `0` to obtain an ephemeral port. Avoid shared fixed ports in
parallel-capable suites.

The owner that spawns a daemon, relay, or shim must:

1. retain its exact handle;
2. poll readiness to a deadline;
3. terminate the exact process;
4. wait for exit;
5. reclaim owned sockets and data;
6. preserve bounded evidence on failure.

Do not use broad process-name killing.

## Time and eventual consistency

Use:

```text
repeat observation
until expected evidence appears or deadline expires
```

The observation should name the eventual fact: relay event, socket connection,
roster entry, process exit, or harness capture.

A fixed sleep may be part of a controlled failure fixture, but it must not be
the assertion that behavior completed. On timeout, report the last observation
and relevant log tail.

Keep deadlines proportional to the boundary. Pure tests should not wait.
Local process startup may need seconds. Provider live labs may need longer, but
must still have an explicit bound.

## External fixture versions

Pin external regression fixtures. Current BDD CI pins Croissant source, Go, and
`nak`. A moving branch or host-installed binary makes a failure ambiguous.

Live labs intentionally test current external systems; record their resolved
versions in the report.

## Failure retention

Successful temporary state should be removed. Failed BDD worlds are copied to
`target/bdd-artifacts`. Other process tests should print or retain an explicit
path when evidence is needed.

Before retaining a directory, check that it contains no provider tokens or
private Nostr keys. Prefer sanitized command output, public event data, and
bounded logs.

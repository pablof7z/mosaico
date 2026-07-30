# Process, relay, and harness tests

Mosaico's defining risks cross process and fabric boundaries. These tests prove
the exact binary, daemon, supervisors, native executables, and relays rather
than an in-process approximation.

## Exact binary rule

Rust integration and BDD targets use:

```rust
env!("CARGO_BIN_EXE_mosaico")
```

Never search `PATH`, invoke nested Cargo, or fall back to an installed Mosaico
binary. Set `MOSAICO_BIN` only where the daemon-spawn contract requires the
exact test binary.

The test must fail if the binary under test cannot be launched. A host install
must not make the test accidentally pass.

## Process ownership

Each world or test owns:

- exact child handles or PIDs;
- private homes, config, sockets, logs, and workspaces;
- a bounded readiness check;
- orderly stop, kill fallback, wait, and cleanup;
- failure output that identifies the first broken boundary.

Never kill by bare process name. Mosaico daemon and PTY supervisors use the same
binary; broad killing can destroy the session behavior under test.

## Relay choice

Use `nak serve` when plain Nostr event flow is sufficient. It is cheap and
leaves no NIP-29 group state.

Use external Croissant when the claim requires NIP-29 create/edit/member state,
relay-signed group projections, or cross-backend discovery. Supply it through
`NIP29_RELAY_BIN`. Mosaico does not build or own Croissant as product
infrastructure.

CI pins Croissant commit
`9c4c93e84852bd9aa6824060b74c56ab2ce812c2`, Go `1.25.12`, and `nak`
`v0.20.0` for deterministic BDD evidence.

Use ephemeral loopback ports and isolated relay data. Readiness means a bounded
connection succeeds and the child remains alive, not that a fixed delay
elapsed.

## Independent relay witnesses

When proving publication or cross-backend discovery, query the relay
independently with `nak` or observe from another isolated backend. Do not accept
the publishing backend's local store as proof that the relay accepted or
shared the event.

Correlate:

- author public key;
- event kind and relevant tags;
- target channel or session;
- event identifier when delivery is under examination;
- receiving harness capture or public command result.

Do not print private keys while producing this evidence.

## Harness shims

A deterministic harness shim is a process boundary double. It may record:

- exact argv;
- selected profile;
- working directory and allowed environment metadata;
- prompts or delivered input;
- lifecycle start, resume, and exit;
- controlled protocol responses.

It must not implement Mosaico's routing or identity decision. If the shim
chooses the expected result on Mosaico's behalf, the test becomes circular.

Use real provider processes only in the opt-in live tier.

## Daemon and supervisor evidence

Assert product-relevant process invariants:

- one configured backend socket has one daemon owner;
- concurrent clients converge on the daemon;
- version replacement targets the daemon without reaping supervisors;
- an admitted runtime's exact endpoint receives addressed input;
- cleanup leaves no owned process or socket.

Detailed race mechanics belong in integration tests. Product-readable outcomes
belong in BDD when operators or agents depend on them.

## Failure artifacts

The BDD world copies failed sandboxes to
`target/bdd-artifacts/<scenario>/`. Process tests should likewise retain or
print bounded log tails when startup, readiness, or delivery fails.

Artifacts must identify binary path, relay URL, backend name, command, exit
status, and relevant logs. Scrub Nostr secrets and provider credentials.

## Commands

```sh
just test-local-relay
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-local-nip29
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-bdd
```

Use the narrowest command while developing, then run every owning deterministic
suite before handoff.

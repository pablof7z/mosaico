# Build, test, and quality

Prefer the repo's `just` recipes over ad-hoc cargo invocations when a recipe
exists. Full suite ownership and CI shape:
[`testing/INDEX.md`](testing/INDEX.md) and
[`testing/ci-and-local-commands.md`](testing/ci-and-local-commands.md).

## Common entry points

```bash
just fmt-check
just lint
just loc-check
just test-unit
just test-hermetic-integration
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-local-nip29
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-behavior-contracts
NIP29_RELAY_BIN=/absolute/path/to/croissant just test   # full local aggregate
```

## Croissant

Croissant is an **external** binary. Mosaico never builds or owns it. Resolve
via `MOSAICO_DEV_CROISSANT_BIN`, `NIP29_RELAY_BIN`, or `croissant` on PATH.

## Behavior contracts

Admit the claim and oracle before implementation; do not silently weaken tests
around code. See the testing index.

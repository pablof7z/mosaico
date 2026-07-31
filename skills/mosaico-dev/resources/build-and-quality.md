# Build, test, and quality

**Why:** shared recipes keep red/green loops comparable and keep CI honest.

**When:** any code change that needs compile, lint, LOC, unit, hermetic, or
local relay/contract evidence.

Prefer the repo's `just` recipes over ad-hoc cargo invocations when a recipe
exists. Full suite ownership and CI shape:
`skills/mosaico-dev/resources/testing/INDEX.md` and
`skills/mosaico-dev/resources/testing/ci-and-local-commands.md`.

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
around code. See `skills/mosaico-dev/resources/testing/INDEX.md`.

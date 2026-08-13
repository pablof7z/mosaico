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

## Quality-gate honesty

A repository or quality gate is a test. Its green result is evidence only when
the gate:

- scans a reproducible intended corpus, normally files tracked by git;
- excludes ignored or generated artifacts unless those artifacts are the
  explicit subject of the check;
- runs in clean CI without relying on workstation-only tools, files, or state;
- has a mutation or self-test that introduces the forbidden condition, proves
  red, removes it, and proves green.

Resolve the corpus before applying the rule so an unrelated local artifact
cannot fail the gate and a missing generated file cannot make it vacuously
green. Pin or provision every required tool in the documented execution
environment. If a check cannot fail for the defect it names, repair it or
delete it; do not retain ceremonial green.

# Deterministic behavior contracts

The former shell E2E rig was replaced by ordinary Rust integration tests plus a
small Cucumber suite for admitted cross-boundary product contracts.

Run the Cucumber contracts with:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-behavior-contracts
```

See [`docs/bdd/000-bdd-approach.md`](../docs/bdd/000-bdd-approach.md) for the
admission rule and runner mechanics. See
[`skills/mosaico-dev/resources/testing/INDEX.md`](../skills/mosaico-dev/resources/testing/INDEX.md)
for the complete evidence architecture, including ordinary tests, adapter
conformance, seeded fault schedules, agent evaluations, and live probes.

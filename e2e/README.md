# Acceptance harness moved

The former shell E2E rig and prose BDD matrix have been replaced by the
executable Gherkin suite under `features/`.

Run it with:

```sh
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-bdd
```

See [`docs/bdd/000-bdd-approach.md`](../docs/bdd/000-bdd-approach.md) for the
runner architecture, tags, fixture contract, and authoring rules.

# Unit tests

Unit tests prove local Mosaico rules with the smallest useful owned boundary.
“Unit” means a coherent behavior unit, not necessarily one function and not
necessarily a mock-isolated class.

## Use unit tests for

- CLI argument and config validation;
- path, identity, locator, and selection rules;
- state-machine transitions and reconciliation policy;
- event encoding/decoding rules;
- rendering and presentation logic;
- migration transformations with controlled fixtures;
- boundary conditions and equivalence classes;
- error classification that does not require a running process.

Examples already live throughout `src/**/tests*`, including state, identity,
fabric context, harness config, reconciliation, session lifecycle, and CLI
rendering.

## Authoring rules

Test a rule, not the current sequence of helper calls. Name the test after the
distinguishing behavior:

```rust
#[test]
fn explicit_session_anchor_overrides_ambient_hint() {
    // Arrange controlled Mosaico inputs, execute the rule, assert the anchor.
}
```

Use the real owned types and pure functions when practical. A fake clock,
temporary directory, or controlled input record is preferable to a mock that
expects an internal call order.

Cover:

- the normal case;
- the boundary values;
- rejected or ambiguous inputs;
- idempotency where repeated events are expected;
- ordering or precedence rules;
- invariants after failure.

Table-driven cases are useful when one rule has several input classes. Split
tests when failures would otherwise be hard to interpret.

Use property or generative tests when the rule spans a broad input space:

- identity and path normalization;
- event encode/decode round trips;
- precedence and ordering invariants;
- idempotent materialization;
- state-machine transitions;
- bounded rendering and secret scrubbing.

Record failing seeds and shrink to the smallest counterexample when the
framework supports it.

## State and filesystem tests

Temporary SQLite stores and temporary homes can still be unit/subsystem tests
when the claim is an owned persistence rule. Inspecting rows is valid here
because the store contract is the subject.

Keep fixtures minimal and explicit. Do not reuse host `~/.mosaico`, installed
provider state, or a public relay. Restore process-global environment through
an owned guard if environment mutation is unavoidable.

## What unit tests do not prove

A unit test of selector mapping does not prove the exact Claude process argv.
A store test does not prove a second backend discovers relay state. A codec
round-trip does not prove a relay accepted the event.

Add broader evidence only when the product or seam claim requires that broader
witness. Do not dismiss the unit test after adding BDD; it still localizes the
cause and covers more edge cases cheaply.

## Regression placement

For a defect in a local rule:

1. Write the failing causal unit test.
2. Confirm it fails for the reported reason.
3. Let the implementation agent make it pass.
4. Add BDD only if an externally meaningful promise was missing.

Avoid asserting private intermediate values merely because they are easy to
reach. Assert the unit's stable input/output or invariant. If a refactor that
preserves the rule breaks the test, the test is probably coupled too tightly.

When the rule or feature itself is deliberately removed, delete its tests and
fixtures. Do not add a unit test whose only claim is that the former rule no
longer exists.

## Running

The repository's hermetic library suite is:

```sh
just test-unit
```

It runs `cargo test --lib`. Focus a test during development with Cargo's name
filter, then run the owning suite before handoff.

## Anti-patterns

- One test for every function regardless of risk.
- Reproducing an entire BDD scenario through internal calls.
- Mocking the exact collaborator call sequence.
- Using production homes, identities, or relays.
- Accepting snapshots without reviewing the semantic change.
- Weakening an assertion because implementation chose another result.
- Treating test count or line coverage as proof of correct behavior.
- Keeping tombstone tests for deliberately deleted features.

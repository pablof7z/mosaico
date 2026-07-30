# Change and regression workflow

Mosaico changes begin with behavior contracts and admitted oracles.
Production code follows.

## Default agent handoff

The design or architecture agent:

1. states the behavior and why it matters;
2. chooses the narrowest authoritative witness;
3. writes the failing unit/property, adapter, black-box, fault, evaluation, or
   architecture oracle;
4. proves the failure is caused by the missing or broken behavior;
5. hands the claim and relevant evidence to a separate implementation agent.

The implementation agent:

1. reads the claim without changing it;
2. adds narrower tests needed to shape local rules;
3. makes the smallest coherent production change;
4. runs focused tests, then all owning suites;
5. returns evidence, artifacts, and any challenge to the original claim.

An adversarial agent then generates contrast cases, failure schedules, and
shortcut implementations that might falsely pass. The designer reviews whether
green still means the intended behavior.

If only one agent is available, complete and review the claim phase before
editing production code. Do not author the expected result around the finished
implementation.

## Challenging a claim

An implementation agent should challenge a test when it:

- contradicts current Mosaico product doctrine;
- asserts an implementation detail rather than behavior;
- uses a witness without authority;
- is impossible to isolate safely;
- encodes an obsolete or removed surface;
- conflicts with a stronger invariant.

The challenge must include concrete source/runtime evidence and a replacement
claim or oracle. Do not silently loosen equality, remove a negative assertion,
add a sleep, or exclude a failing contract.

## Oracle authority and reopening

Discovery is allowed to show that a proposed contract is wrong, incomplete, or
conceptually confused. Once admitted, the oracle is authoritative for the
implementation pass, not permanently frozen.

Reopening requires explicit contract discussion with the new evidence and the
consequence for the product claim. The implementation agent may initiate that
discussion, but does not edit the oracle first and present the weaker result as
success.

## New product behavior

Start with examples that distinguish the rule:

- useful successful path;
- important rejection or ambiguity;
- relevant must-never failure;
- identity, authority, durability, and secrecy effects.

Use Cucumber only if the stable deterministic product promise passes every
admission rule. Add unit/property and adapter tests for local policies and
technical semantics. Define evaluation datasets instead when the desired
behavior depends on a model choosing among several valid trajectories.

Evaluate every proposed scenario independently. Inclusion in an issue,
inventory, or initial-suite list is not admission. If Gherkin does not
materially clarify the claim, keep the same public behavior in an ordinary
real-binary integration test.

## Bug fix

1. Reproduce the causal defect at the narrowest honest layer.
2. Confirm the test fails on the current defect.
3. Determine whether an existing broader contract, fault invariant, or
   capability evaluation should also have caught it.
4. Add broader evidence only when that evidence family owns a missing claim.
5. Implement without weakening either claim.
6. Run adjacent suites that exercise the same authority boundary.

Not every bug needs Gherkin. A parser edge case with an adequate product
contract normally needs only the causal unit regression.

## Refactor

Existing behavior contracts should remain unchanged. Add characterization
tests only where the current technical seam is unclear or risky.

Refactoring is successful when:

- public behavior remains green;
- lower-level tests move with the ownership boundary;
- obsolete tests and helpers disappear;
- no compatibility shim is introduced;
- failure diagnostics remain at least as strong.

Do not preserve a source layout merely because tests assert private calls.

## Feature removal

When a feature, command, option, compatibility surface, or behavior is
deliberately removed:

1. Delete its Gherkin scenarios and exclusive step definitions.
2. Delete its unit, integration, contract, and process tests.
3. Delete fixtures, doubles, snapshots, and helpers that exist only for it.
4. Keep or rewrite a nearby test only when it expresses an independent current
   Mosaico rule.

Do not add a negative test proving that the deleted feature no longer exists.
Do not preserve its vocabulary as a forbidden-name or rejected-alias test. The
current suite specifies the current product; Git history records removed
behavior.

## Protocol, config, or CLI change

Write exact accepted and rejected cases for the new current contract. Delete
tests and fixtures for any deliberately removed surface in the same change.
Add or update a black-box behavior contract only when the new current outward
claim passes its admission rule.

Mosaico does not retain backwards compatibility. Tests specify the current
surface rather than memorializing stale input.

## Concurrency and lifecycle change

Express the product promise separately from the schedule:

- black-box contract, if admitted: clients observe one daemon-owned backend;
- integration/process test: spawn, lock, socket, and writer mechanics;
- seeded schedule test: varied client ordering, death, and restart points.

Use bounded stress to search, then preserve failures as replayable schedules.
Never make a precise thread interleaving the contract unless the protocol
defines it.

## Emergent agent behavior

Before implementation, define:

- controlled tasks and repositories;
- no-Mosaico and relevant feature conditions;
- repetitions, models/harnesses, and budgets;
- outcome, trajectory, cost, and latency metrics;
- independent scoring and artifact retention.

Do not claim that one successful agent conversation proves a capability.

## Committed Cucumber state

Every scenario on `master` executes deterministically. Future behavior and
known bugs remain in GitHub Issues until implementation work begins. Do not use
`@designed`, `@wip`, `@live`, historical migration, or issue tags as a second
planning catalog.

## Completion evidence

Handoff includes:

- claims added or changed;
- focused failing-before/passing-after evidence;
- adversarial cases and oracle challenges considered;
- owning deterministic suites run;
- seeds/replay commands or evaluation run sets when relevant;
- live/probe evidence only when relevant;
- retained artifact paths for failures;
- explicit unrun suites and dependencies;
- one GitHub issue for any remaining actionable gap.

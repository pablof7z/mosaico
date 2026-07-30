# Change and regression workflow

Mosaico changes begin with executable claims. Production code follows.

## Default agent handoff

The design or architecture agent:

1. states the behavior and why it matters;
2. chooses the narrowest authoritative witness;
3. writes the failing BDD, unit, contract, or architecture claim;
4. proves the failure is caused by the missing or broken behavior;
5. hands the claim and relevant evidence to a separate implementation agent.

The implementation agent:

1. reads the claim without changing it;
2. adds narrower tests needed to shape local rules;
3. makes the smallest coherent production change;
4. runs focused tests, then all owning suites;
5. returns evidence, artifacts, and any challenge to the original claim.

The designer reviews whether green still means the intended behavior.

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
claim or observer. Do not silently loosen equality, remove a negative
assertion, add a sleep, or mark the scenario `@wip`.

## New product behavior

Start with examples that distinguish the rule:

- useful successful path;
- important rejection or ambiguity;
- relevant must-never failure;
- identity, authority, durability, and secrecy effects.

Write BDD when this is a stable operator/agent promise. Add unit and contract
tests for the local policies and protocol shapes the implementation needs.

## Bug fix

1. Reproduce the causal defect at the narrowest honest layer.
2. Confirm the test fails on the current defect.
3. Determine whether an existing feature scenario should also have failed.
4. If the product contract was absent, add the missing BDD example.
5. Implement without weakening either claim.
6. Run adjacent suites that exercise the same authority boundary.

Not every bug needs Gherkin. A parser edge case with an adequate product
contract normally needs only the causal unit regression.

## Refactor

Existing acceptance contracts should remain unchanged. Add characterization
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

1. Delete its BDD scenarios and exclusive step definitions.
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
Add or update BDD if operators or agents observe a different current command,
error, identity, or lifecycle.

Mosaico does not retain backwards compatibility. Tests specify the current
surface rather than memorializing stale input.

## Concurrency and lifecycle change

Express the product promise separately from the schedule:

- BDD: concurrent clients still observe one daemon-owned backend.
- Integration/process test: spawn race, lock, socket, and writer mechanics.

Use bounded stress where it increases confidence. Never make a precise thread
interleaving the contract unless the protocol defines it.

## Excluded scenario lifecycle

`@designed` and `@wip` scenarios must reference an open, behavior-specific
issue. An umbrella implementation issue is not durable ownership.

Closing the issue requires one of:

- remove the exclusion and prove the scenario passes;
- move the scenario to a different valid open issue with justification;
- correct or remove a contract that is no longer intended.

Current use of `@issue-704` by several designed product gaps is known debt, not
a template.

## Completion evidence

Handoff includes:

- claims added or changed;
- focused failing-before/passing-after evidence;
- owning deterministic suites run;
- live/probe evidence only when relevant;
- retained artifact paths for failures;
- explicit unrun suites and dependencies;
- one GitHub issue for any remaining actionable gap.

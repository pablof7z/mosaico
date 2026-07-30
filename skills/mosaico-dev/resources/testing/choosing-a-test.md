# Choosing a test

Use this sequence before choosing a file or framework.

## 1. Write the claim

Use one sentence:

> When an operator addresses an offline stable agent, Mosaico starts that agent
> under its configured public identity and delivers the message once.

If the sentence names a Rust function rather than an operator, agent, protocol
peer, or owned subsystem, it is probably not a product claim.

## 2. Name the consequence

Ask what breaks if the claim is false:

- visible product behavior;
- identity, authority, secrecy, or durability;
- a local rule;
- a technical seam;
- compatibility with an external system;
- repository maintainability.

High-cost identity, authority, message-loss, process-ownership, and secret
failures normally deserve both an acceptance witness and narrow causal tests.

## 3. Choose the observer

Use the first observer with enough authority:

1. Pure return value or state transition: unit test.
2. Owned store, codec, RPC, or adapter boundary: integration/contract test.
3. Exact binary, daemon, socket, child process, or local relay: process test.
4. Supported CLI/relay/harness outcome stated in product language: BDD.
5. Real provider or public relay behavior: opt-in probe/live lab.

Do not move outward merely because a broader test feels more realistic.

## 4. Check determinism

Routine evidence must control:

- homes, config, identities, ports, and working directories;
- binary and external fixture versions;
- child-process ownership and cleanup;
- clock and eventual-consistency deadlines;
- provider responses or harness behavior.

If real credentials, public infrastructure, rate limits, or model output are
required, the check belongs in the live tier. If the unknown can be learned
once and represented by a pinned local fixture, do that for regression.

## 5. Decide whether BDD is warranted

Add or change a feature scenario when the claim:

- is stable enough to be a product promise;
- is meaningful to an operator or participating agent;
- crosses a boundary where local tests can pass while Mosaico is broken;
- protects a must-never safety or authority rule;
- resolves an ambiguity that future implementers might interpret differently.

Do not use BDD for:

- every parser branch;
- schema columns or migration statements;
- internal RPC choreography;
- race mechanics with no readable product-level outcome;
- current repository architecture or file-layout constraints;
- a full permutation matrix already covered by a local rule test.

## Change-specific defaults

New product behavior:

- author examples and a failing executable product claim first;
- add narrow unit/contract tests for the rules needed to implement it.

Bug:

- reproduce the cause at the narrowest honest boundary;
- add BDD only if the defect violated a missing or inadequate product contract.

Refactor:

- preserve existing product contracts;
- add lower-level characterization only where the refactor crosses an unclear
  seam;
- do not invent new BDD when public behavior is unchanged.

Feature removal:

- delete every scenario, test, fixture, and helper owned only by the removed
  behavior;
- do not add a negative test proving the deleted surface remains absent;
- keep a nearby test only when it states an independent current Mosaico rule.

Protocol or config change:

- add contract tests for exact accepted and rejected shapes;
- add BDD when operators or agents observe different behavior.

Concurrency or process lifecycle:

- use integration/process stress and exact process evidence;
- add BDD for the stable visible promise, not for thread scheduling.

External uncertainty:

- write a bounded probe with explicit side effects;
- convert the learned invariant into deterministic evidence where possible.

## Duplication test

Before adding another layer, ask:

- Can the existing test pass while this behavior is broken?
- Will the new witness locate a different class of failure?
- Does the new claim communicate a stable product rule?

If all answers are no, strengthen or relocate the existing test.

## Exception record

When the default cannot work, write a short note in the test or durable owning
guide explaining the Mosaico constraint, lost confidence, and compensating
evidence. Do not create a GitHub planning duplicate; actionable follow-up
belongs in one issue.

# Testing philosophy

Testing makes a Mosaico claim falsifiable and collects evidence from an oracle
with authority to decide whether the claim is true.

## Claims, oracles, and boundaries

Every useful check answers:

1. What current behavior, rule, or capability is claimed?
2. What failure matters?
3. Which oracle can distinguish success from a convincing imitation?
4. At what boundary can that oracle observe the claim?
5. Is the claim deterministic, schedule-dependent, model-dependent, or live?

“A second backend sees the workspace” is a claim. That backend's public
listing, an independent relay query, and filesystem isolation are authoritative
evidence. The publishing backend's SQLite row is not.

“Claude receives the configured native selector” is an adapter claim. A typed
driver contract or deterministic process capture is authoritative. It does not
need product prose or a real model.

## BDD is a discipline

BDD means discovering behavior through concrete examples, counterexamples, and
shared language before implementation. It does not mean that every example
becomes Gherkin.

Cucumber can preserve a few critical examples as executable product-readable
contracts. Ordinary Rust can also be behavior-driven and black-box. Choose the
representation that makes the oracle clearest and cheapest to maintain.

## Deterministic fabric versus emergent capability

Fabric behavior is controlled enough for pass/fail contracts:

- identity continuity;
- message durability and delivery bounds;
- relay-only discovery;
- authorization and secrecy;
- daemon and hook ownership;
- restart reconciliation.

Agent capability depends on model, prompt, context, peers, and available valid
trajectories:

- noticing overlapping work;
- routing a finding to the useful actor;
- coordinating without duplicate implementation;
- opening a useful subchannel;
- improving collective task quality.

Do not turn the second group into one deterministic path. Evaluate it over
repeated runs, diverse tasks, multiple harness/model combinations, and a
baseline.

## The evidence portfolio

Mosaico needs complementary evidence:

- unit/property tests explore local rules and broad input spaces;
- adapter conformance tests prove implementations preserve one typed semantic
  contract;
- integration/process tests prove owned seams and failure mechanics;
- a small black-box suite proves critical deterministic product promises;
- seeded fault tests explore asynchronous schedules reproducibly;
- capability evaluations score emergent outcomes and trajectories;
- live probes prove current external compatibility;
- quality gates constrain source and repository structure.

No family is prestigious. Production likeness is not automatically stronger:
the best evidence is the least variable oracle with enough authority.

## Independence

The same agent writing the requirement, oracle, implementation, and assessment
in one uninterrupted context can narrow the claim around its own code.

Mosaico's default is:

- design agent: behavior contract and initial oracle;
- implementation agent: production change and supporting local tests;
- adversarial agent: contrast cases, failure schedules, and false-pass search;
- design review: explicit approval for any oracle revision.

Independent tests can still be wrong. Clear issue contracts and oracle review
prevent independence from becoming hidden arbitrariness.

## Green and coverage

Never optimize for a green dashboard, scenario count, or coverage percentage.
Those measure evidence machinery, not the importance or truth of its claims.

A skipped scenario is no evidence. A test that cannot fail for its named defect
is false confidence. A test made green by weakening the oracle is a regression.
Coverage may reveal unexamined code, but risk, contract, schedule, and
capability coverage drive decisions.

## Product lifetime

Tests live only while their behavior lives. Delete tests and fixtures for a
removed feature. Keep a nearby test only if it independently specifies current
behavior without naming the removed concept.

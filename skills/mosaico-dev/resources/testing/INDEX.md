# Mosaico testing guide

This guide is for agents designing, implementing, debugging, or reviewing
Mosaico. Testing begins with a claim and an oracle, not with a framework.

## North star

Mosaico uses **Behavior-Contract-Driven Development**:

1. State the current behavior, examples, counterexamples, and must-never
   consequences before production implementation.
2. Admit an oracle that can falsify the claim before implementation begins.
3. Normally separate oracle authorship from implementation.
4. Run an adversarial pass against shortcuts and missing contrast cases.
5. Keep the smallest evidence portfolio that proves the current product.

Green is evidence that a meaningful claim passed. Green is not the objective
by itself.

BDD is the discovery and specification discipline. Cucumber is only a narrowly
admitted executable/reporting surface; `.feature` files are not the foundation
or complete catalog of Mosaico testing.

## Evidence families

| Need to resolve | Strong default |
|---|---|
| Local rule, transformation, or input space | Rust unit, property, or generative test |
| Relay, harness, transport, or provider-adapter equivalence | Shared typed conformance suite |
| Critical deterministic product promise across boundaries | Black-box behavior contract; selectively Cucumber |
| Race, restart, duplication, reordering, or partial failure | Seeded replayable fault/schedule test |
| Model-dependent awareness or coordination capability | Repeated comparative agent evaluation |
| Current real-provider or public-network compatibility | Explicit opt-in probe or live lab |
| Repository structure or current architecture boundary | Quality or architecture gate |

These are not ranks. Each family answers a different uncertainty.

## Start here

For every change:

1. Read [Testing philosophy](philosophy.md).
2. Use [Choosing evidence](choosing-a-test.md).
3. Follow [Change and regression workflow](change-and-regression-workflow.md).

Then route to the owning guide:

- [Unit tests](unit-tests.md)
- [Integration and adapter contracts](integration-and-contract-tests.md)
- [Process, relay, and harness tests](process-relay-and-harness-tests.md)
- [Deterministic behavior contracts and BDD](bdd.md)
- [Writing admitted Gherkin scenarios](writing-gherkin-scenarios.md)
- [Seeded fault and schedule testing](seeded-fault-and-schedule-testing.md)
- [Agent capability evaluations](agent-capability-evaluations.md)
- [Probes, validation seeds, and live labs](probes-seeds-and-live-labs.md)
- [Fixtures, doubles, isolation, and time](fixtures-doubles-isolation-and-time.md)
- [Failures, flakiness, and artifacts](failures-flakiness-and-artifacts.md)
- [CI and local commands](ci-and-local-commands.md)
- [Worked Mosaico examples](worked-mosaico-examples.md)
- [Review checklists](review-checklists.md)

## Oracle workflow

The design or architecture agent normally writes the issue contract and the
first executable oracle. A separate implementation agent makes it pass.

The implementation agent may challenge an inaccurate or overconstrained
oracle with evidence, but must not silently weaken it. A fresh adversarial pass
then asks:

- What shortcut implementation could falsely pass?
- Which counterexample distinguishes the intended rule?
- Which failure boundary remains uncontrolled?
- Does the oracle reject another valid implementation or trajectory?

The oracle need not be hidden. Independence comes from authorship/review
separation and explicit contract changes.

Oracle authority begins after discovery and admission. It means the
implementation must satisfy the admitted claim; it does not make the oracle
immune to correction. New evidence may reopen the claim through explicit
contract discussion. Until that discussion resolves, the implementation agent
does not rewrite the oracle around its code.

## Product lifetime

Tests specify current behavior. When Mosaico deliberately removes a feature or
compatibility surface, delete its scenarios, tests, fixtures, and dedicated
helpers in the same change. Do not replace them with tests proving the dead
surface remains absent; Git history records the removal.

## Exception rule

These guides define strong defaults. An agent may bend one only by recording:

1. the rule being bent;
2. the concrete Mosaico constraint;
3. the confidence lost;
4. compensating evidence;
5. why the exception is smaller than changing the design.

“The proper test was difficult to write” is not sufficient.

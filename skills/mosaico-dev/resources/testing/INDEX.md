# Mosaico testing guide

This index is for agents designing, implementing, debugging, or reviewing
Mosaico. Start with the confidence required, not with a framework or a desired
test count.

## The governing rule

State the claim, name its authoritative witness, and use the narrowest Mosaico
boundary that can prove it honestly.

Mosaico development is test-driven:

1. The design or architecture agent normally writes the executable claim first.
2. A separate implementation agent makes that claim pass.
3. The implementation agent may challenge a false or overconstrained claim,
   but must not silently weaken it.
4. Green is evidence that a meaningful claim passed. Green is not the objective
   by itself.

If separate agents are impractical, explicitly separate the claim-authoring and
implementation phases and review the claim before changing production code.

Tests exist only for current behavior. When Mosaico deliberately removes a
feature or compatibility surface, delete its scenarios, fixtures, and
lower-level tests in the same change. Do not replace them with tests proving
that the dead feature remains absent; version history records the removal.

## Two questions, not one ladder

Classify every proposed test on two independent axes.

First ask what kind of claim it makes:

- product behavior for an operator or agent;
- a local rule or transformation;
- a technical boundary contract;
- an experiment about external infrastructure;
- a repository or architecture constraint.

Then ask what execution boundary can witness it:

- function or module;
- store or subsystem;
- binary, daemon, socket, or child process;
- local `nak` or Croissant relay;
- real provider or public infrastructure.

BDD is on the first axis: product-facing intent and shared language. Unit,
integration, process, and end-to-end describe the second axis. A BDD scenario
can drive a process boundary, but process testing is not automatically BDD.

## Quick selection

| Confidence needed | Strong default |
|---|---|
| Stable Mosaico product promise | BDD feature and supported public witness |
| Local parser, renderer, policy, or state transition | Unit test |
| Store, RPC, codec, transport, or subsystem seam | Integration or contract test |
| Daemon ownership, process lifecycle, relay flow, or exact native argv | Process/relay/harness test |
| Unknown behavior of a live relay or provider | Probe or live lab |
| Shell, container, skill script, or site behavior | Tooling test |
| Formatting, lint, file size, or current architecture boundary | Quality/architecture gate |

Regression is not a separate layer. Put the causal regression at the narrowest
honest boundary. Add BDD only when the defect exposed an uncovered product
promise.

## Reading routes

For a new behavior:

1. [Philosophy](philosophy.md)
2. [Choosing a test](choosing-a-test.md)
3. [BDD](bdd.md) and [writing Gherkin scenarios](writing-gherkin-scenarios.md)
4. [Change and regression workflow](change-and-regression-workflow.md)

For implementation mechanics:

- [Unit tests](unit-tests.md)
- [Integration and contract tests](integration-and-contract-tests.md)
- [Process, relay, and harness tests](process-relay-and-harness-tests.md)
- [Fixtures, doubles, isolation, and time](fixtures-doubles-isolation-and-time.md)

For uncertain or real infrastructure:

- [Probes, seeds, and live labs](probes-seeds-and-live-labs.md)
- [Failures, flakiness, and artifacts](failures-flakiness-and-artifacts.md)

For execution and review:

- [CI and local commands](ci-and-local-commands.md)
- [Worked Mosaico examples](worked-mosaico-examples.md)
- [Review checklists](review-checklists.md)

## Evidence portfolio

The current repository uses:

- Rust library tests under `src/**/tests*`;
- integration targets under `tests/`;
- executable Cucumber contracts under `features/` and `bdd/`;
- shell tests under `skills/mosaico-dev/tests` and `scripts/tests`;
- Node site tests under `site/`;
- ignored live probes under `tests/*probe.rs`;
- `mosaico-dev` container labs for real provider proof;
- formatting, Clippy, LOC, and helper-import gates.

Do not translate every lower-level test into Gherkin. Do not replace a product
contract with a source search. Keep complementary evidence when it answers a
different failure question.

## Exceptions

These guides define strong defaults. An agent may make an exception only when
it records:

1. the rule being bent;
2. the concrete Mosaico constraint;
3. the confidence lost;
4. the compensating evidence;
5. why the exception is smaller than changing the design.

“The test was difficult to write” is not sufficient.

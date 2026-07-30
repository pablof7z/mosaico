# Seeded fault and schedule testing

Mosaico is asynchronous distributed software. Example-based tests cover known
paths; seeded schedule tests explore when the same events arrive in different
orders or components fail between durable transitions.

This is a target architecture. Add controllable seams incrementally around real
bugs and high-risk state machines rather than attempting a complete virtual
cluster at once.

## Risks this family owns

- relay disconnect before or after publish acknowledgement;
- daemon death between durable writes;
- duplicate, delayed, reordered, or replayed events;
- profile metadata arriving after an addressed message;
- PTY or ACP endpoint disappearance during delivery;
- simultaneous clients and sessions;
- stale presence and lease expiry;
- partial relay availability;
- restart during inbox claim, launch, or receipt publication.

Do not enumerate these as dozens of Gherkin timing scripts.

## Required properties

Every schedule test must have:

- a recorded seed;
- controlled fault points or event ordering;
- deterministic randomness from that seed;
- explicit invariants rather than expected thread choreography;
- bounded execution;
- a replay command;
- a minimized or at least preserved failing artifact.

A failure report must identify the seed, scenario/workload, injected faults,
event order, first violated invariant, and retained state/log paths.

## Useful invariants

- one public identity never has two active sibling runtimes;
- an accepted message remains pending, delivered, or explicitly failed;
- delivery is no more than once when that is the contract;
- stale generations cannot revoke current runtimes;
- restart cannot erase a durable claim or ownership fence;
- duplicate relay events are idempotent;
- one backend home converges on one daemon/store writer;
- secrets and authority never cross untrusted boundaries.

## Incremental implementation

Start at owned boundaries:

1. Extract deterministic transition functions and property-test them.
2. Add seeded ordering to fake relay/event queues.
3. Add named failpoints around durable writes and acknowledgements.
4. Run small state-machine workloads across many seeds.
5. Preserve and replay every discovered failure.
6. Add process death/restart only after in-process schedules are reproducible.

Use real serialization, state transitions, and owned stores where practical.
Fake time, network delivery, and child endpoints only at boundaries whose
variability the test intentionally controls.

## Relationship to regressions

When a production race is found:

- keep the smallest causal unit/state-machine regression;
- add its exact seed or minimized schedule to the replay suite;
- keep a black-box behavior contract only for the stable outward promise;
- do not add one Gherkin scenario per observed interleaving.

## CI shape

Fast fixed seeds may become required PR CI. Larger randomized campaigns belong
in scheduled CI until cost and reliability justify promotion.

Never call an unreplayable stress run deterministic evidence. Repetition is a
search technique; the saved seed and invariant make the result actionable.

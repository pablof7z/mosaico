# Choosing evidence

Choose the evidence family before choosing a file or framework.

## 1. State the claim

Use one sentence:

> Work addressed to an offline stable agent survives daemon restart, remains
> bound to the same public key, and is delivered no more than once.

Name a current Mosaico rule, product outcome, adapter semantic, failure
invariant, or measurable capability.

## 2. Classify the uncertainty

### Local deterministic rule

Examples: selector precedence, event decoding, path resolution, reconciliation
transition, authorization decision.

Use a unit/property test. Include equivalence classes and counterexamples.

### Adapter equivalence

Examples: every PTY driver applies its native profile selector; every hosted
transport preserves identity and lifecycle outcomes.

Use one typed conformance suite against every implementation. Add a process
capture only when the executable boundary itself is the contract.

### Critical deterministic product promise

Examples: relay-only cross-backend discovery, no sibling identity on resume,
hook fail-open authority, exactly bounded addressed delivery.

Use the actual binary and public/independent witnesses. Prefer Rust black-box
tests. Admit Cucumber only when product-language examples add durable clarity.

### Asynchronous schedule risk

Examples: daemon death between durable writes, duplicate relay events, delayed
ACK, stale presence, simultaneous sessions.

Use a seeded fault/schedule harness with replay artifacts. One hand-authored
happy-path scenario is insufficient.

### Emergent agent capability

Examples: noticing overlap, useful peer contact, avoiding duplicate work,
collective task quality.

Use repeated comparative evaluations. Score outcomes and flexible trajectory
properties rather than demanding one exact path.

### External compatibility

Examples: current provider auth, public-relay policy, deployed protocol
behavior.

Use an explicit opt-in probe or live lab. Convert stable findings into
deterministic local evidence where possible.

## 3. Choose the oracle

Use the narrowest observer with enough authority:

1. pure result or state transition;
2. typed adapter output;
3. owned store, frame, socket, or process capture;
4. supported CLI/RPC result or independent relay witness;
5. repeated scored agent outcome;
6. real provider or public infrastructure.

Do not move outward merely because a broader test looks more realistic.

## 4. Decide whether Cucumber earns admission

A scenario belongs in `features/` only when every answer is yes:

- Is this a stable operator/agent-visible promise?
- Is it deterministic with local controlled fixtures?
- Can lower-level tests pass while this promise remains broken?
- Is the failure consequence load-bearing?
- Do concrete product-language examples remove real ambiguity?
- Can one or a few examples express the rule without a permutation matrix?
- Is the public or independent oracle clear?
- Is the glue-code maintenance cost justified?

Otherwise use Rust, fault testing, evaluations, or a live probe.

Never use Cucumber for:

- exact adapter matrices or internal protocol choreography;
- parser branches and broad input spaces;
- speculative future behavior or issue backlogs;
- known failing scenarios skipped on `master`;
- model quality or one prescribed coordination trajectory;
- timing permutations;
- removed-feature tombstones.

## Change defaults

New behavior:

- specify examples and counterexamples first;
- author the narrowest failing oracle;
- add a black-box contract only when local evidence lacks authority.

Bug:

- reproduce the cause at the narrowest honest boundary;
- add a broader contract only when a product promise was absent.

Refactor:

- preserve existing behavior evidence;
- add characterization only for unclear seams.

Feature removal:

- delete all evidence owned only by the removed behavior;
- do not create an absence test.

Emergent claim:

- define a dataset, baseline, repetitions, metrics, and artifact policy before
  claiming improvement.

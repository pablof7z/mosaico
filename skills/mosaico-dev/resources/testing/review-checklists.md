# Testing review checklists

Use the relevant checklist before implementation handoff and again before
declaring the change complete.

## Claim design

- [ ] The claim states a real Mosaico rule or behavior.
- [ ] The consequence of failure is named.
- [ ] The witness has authority to decide the claim.
- [ ] The boundary is the narrowest one that can prove it honestly.
- [ ] The claim was executable and failing before production implementation.
- [ ] A separate implementation agent was used, or claim and implementation
      phases were explicitly separated.
- [ ] No assertion was weakened merely to obtain green.

## BDD

- [ ] Feature prose uses operator/agent Mosaico language.
- [ ] The scenario distinguishes one coherent behavior.
- [ ] `Given` contains only relevant state.
- [ ] `When` is the causal product event.
- [ ] `Then` uses public or independent witnesses.
- [ ] The scenario survives an implementation rewrite.
- [ ] No SQLite table, Rust helper, or existing test name appears as outcome.
- [ ] No fixed sleep is treated as proof.
- [ ] The exact Cargo-built Mosaico binary is used.
- [ ] The scenario is isolated and cleans every owned process.
- [ ] `@live` is used only for credentials/public infrastructure.
- [ ] `@designed` or `@wip` has an open behavior-specific issue.
- [ ] An excluded scenario is not reported as passing coverage.

## Unit

- [ ] The test names a local rule, boundary, or invariant.
- [ ] Normal, boundary, rejection, and precedence cases are considered.
- [ ] Assertions target stable input/output behavior.
- [ ] Internal call order is not frozen without a contract reason.
- [ ] Temporary state cannot touch host Mosaico or provider data.
- [ ] A refactor preserving the rule would not require rewriting the test.

## Integration and contract

- [ ] The technical seam under test is explicit.
- [ ] Accepted and rejected contract shapes are covered.
- [ ] Tests and fixtures for deliberately removed surfaces are deleted.
- [ ] No tombstone test exists solely to prove a dead surface remains absent.
- [ ] Real owned components are used where practical.
- [ ] Any double stops at the boundary rather than implementing Mosaico logic.
- [ ] Environment mutation and shared relay state are serialized or isolated.
- [ ] Concurrency assertions target invariants, not a lucky schedule.
- [ ] Broader BDD is added only for a separate product claim.

## Process, relay, and harness

- [ ] `CARGO_BIN_EXE_mosaico` selects the binary.
- [ ] Home, config, workspace, socket, identity, and port are isolated.
- [ ] `nak` versus Croissant matches required protocol semantics.
- [ ] External fixture versions are pinned for deterministic evidence.
- [ ] Readiness and eventual state use bounded polling.
- [ ] Exact child handles are stopped and waited.
- [ ] Relay or harness evidence is independent where required.
- [ ] Logs and artifacts cannot leak secrets.

## Probe or live lab

- [ ] The question cannot be answered deterministically.
- [ ] Relay/provider and side effects are explicit.
- [ ] Credentials and state are isolated.
- [ ] The check proves transport/fabric behavior, not model quality.
- [ ] Versions, public event ids, and cleanup are reported.
- [ ] Stable findings are converted into deterministic evidence where possible.

## Completion

- [ ] Focused red-before/green-after evidence exists.
- [ ] Every owning deterministic suite ran.
- [ ] Adjacent suites sharing the changed authority boundary ran.
- [ ] `fmt-check`, `lint`, and `loc-check` ran when the change is ready.
- [ ] Unrun live or dependency-backed checks are stated explicitly.
- [ ] Failures identify the first broken boundary and artifact path.
- [ ] Test duplication adds a distinct witness or diagnostic value.
- [ ] Removed behavior left no obsolete scenarios, fixtures, or negative tests.
- [ ] Remaining actionable work has one GitHub issue, not a duplicate plan.
- [ ] Green is reported as evidence for named claims, not as the objective.

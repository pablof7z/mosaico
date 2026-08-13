# Testing review checklists

Use the relevant checklist before implementation handoff and again before
declaring the change complete.

## Claim design

- [ ] The claim states a real Mosaico rule or behavior.
- [ ] The consequence of failure is named.
- [ ] The witness has authority to decide the claim.
- [ ] Determinism is classified: deterministic, schedule-dependent,
      model-dependent, or live.
- [ ] The boundary is the narrowest one that can prove it honestly.
- [ ] The claim was executable and failing before production implementation.
- [ ] A separate implementation agent was used, or claim and implementation
      phases were explicitly separated.
- [ ] No assertion was weakened merely to obtain green.
- [ ] A user correction changes the owning claim and adds contrast cases; it
      is not left only in chat or appended as a contradictory claim.

## Cucumber admission and BDD

- [ ] This scenario passes admission independently; its presence in an issue
      or suite inventory is not the reason it exists.
- [ ] Examples and counterexamples were discovered before implementation.
- [ ] The claim is a stable deterministic product promise.
- [ ] Lower-level evidence can pass while the outward promise remains broken.
- [ ] Product-language examples add durable clarity.
- [ ] The failure consequence justifies step-glue maintenance.
- [ ] The claim is not an adapter matrix, timing schedule, model capability,
      live check, future plan, known failure, or tombstone.
- [ ] Delivery-count language names the observable boundary and does not imply
      stronger crash/retry semantics than the oracle proves.
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
- [ ] The committed scenario runs in the required deterministic suite.
- [ ] No planning, issue, live, WIP, or historical tags exist.

## Unit

- [ ] The test names a local rule, boundary, or invariant.
- [ ] Normal, boundary, rejection, and precedence cases are considered.
- [ ] Assertions target stable input/output behavior.
- [ ] Internal call order is not frozen without a contract reason.
- [ ] Temporary state cannot touch host Mosaico or provider data.
- [ ] A refactor preserving the rule would not require rewriting the test.

## Fixtures and test setup

- [ ] Each fixture stages a real input or declared starting state, not the
      internal result the test claims Mosaico derived.
- [ ] The review question was answered: is this input a cause, or the
      conclusion being proved?
- [ ] Direct state insertion is used only when that state is the declared input
      to the narrower rule under test.

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
- [ ] Equivalent adapters run one shared typed conformance suite.
- [ ] Provider-specific cases cover real differences rather than copied
      semantics.

## Seeded fault and schedule

- [ ] The test varies a real asynchronous risk.
- [ ] Randomness and fault points are controlled by a recorded seed.
- [ ] Assertions target invariants, not a lucky interleaving.
- [ ] Failure output contains a replay command and retained artifacts.
- [ ] Fixed seeds are distinguished from exploratory campaigns.

## Agent capability evaluation

- [ ] The claim genuinely depends on model or peer choices.
- [ ] Controlled tasks and starting state are recorded.
- [ ] A no-Mosaico or relevant feature baseline exists.
- [ ] Repetitions and model/harness conditions are explicit.
- [ ] Scoring permits multiple valid trajectories.
- [ ] Outcomes, transcripts, diffs, latency, and cost are retained.
- [ ] The implementation agent is not the sole evaluator.
- [ ] No result is promoted into deterministic CI solely because repeated
      capability runs happened to pass.

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

## Repository and quality gates

- [ ] The gate scans an explicit reproducible corpus, normally tracked files.
- [ ] Ignored and generated artifacts are excluded unless explicitly tested.
- [ ] Every required tool and input exists in clean CI without workstation
      state.
- [ ] A mutation or self-test proves the named defect turns the gate red and
      its removal turns the gate green.
- [ ] A check that cannot fail for its named defect is repaired or deleted.

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

# Failures, flakiness, and artifacts

A failed test is evidence to classify, not an obstacle to suppress.

## First classify the failure

Claim failure:

- Mosaico produced the wrong observable result.
- Keep the failing claim and diagnose production behavior.

Test-model failure:

- The scenario or assertion does not describe intended behavior.
- Return it to design with evidence before changing the claim.

Harness failure:

- The test selected the wrong binary, leaked state, lost a child process, or
  failed to capture the intended witness.
- Repair the harness and show why the product claim was not evaluated.

Fixture/dependency failure:

- `nak`, Croissant, a port, toolchain, or expected executable is unavailable.
- Report the resolved dependency and first failing boundary.

Live-environment failure:

- Provider auth, a public relay, rate limits, or mutable external behavior
  prevented proof.
- Keep it out of deterministic green/red claims and preserve live evidence.

## No blind retries

Do not make a red test green by:

- rerunning until it passes;
- increasing a sleep without identifying the awaited fact;
- swallowing an error or accepting empty output;
- broadening an equality assertion;
- excluding it from the committed behavior-contract suite;
- sharing warmed state from a previous scenario.

A retry belongs inside product or fixture semantics only when the boundary
contract defines retry. The test should still assert the eventual state and
expose attempts or deadline failure.

## Flakiness investigation

Check, in order:

1. shared environment, home, port, socket, relay data, or identity;
2. wrong binary or host executable selected from `PATH`;
3. child readiness and exact process ownership;
4. fixed sleeps hiding eventual consistency;
5. unordered collection or timestamp assumptions;
6. test-order dependence;
7. public infrastructure or provider variability;
8. a real product race.

Repeat a focused test to characterize a suspected race, not to certify it.
Once understood, encode the invariant, preserve a replayable seed/schedule,
and remove timing luck.

## Useful failure output

Report:

- claim and scenario/test name;
- exact Mosaico binary path;
- sanitized command and working directory;
- exit status, stdout, and stderr;
- backend and relay URL;
- expected witness and last observed state;
- bounded daemon/relay/harness log tails;
- child-process exit state;
- artifact directory.

Lead with the first failing boundary. A later missing output is often only a
consequence.

## BDD artifacts

Failed BDD worlds are copied to:

```text
target/bdd-artifacts/<scenario>/
```

Inspect backend stdout/stderr, daemon logs, relay logs, harness captures,
configs, and workspace state. Confirm that retained material does not expose
private keys or provider credentials before sharing it.

Successful worlds should clean up. A leaked daemon, supervisor, relay, socket,
or data directory is a harness failure even if assertions passed.

## Live-lab artifacts

Use the `mosaico-dev` run directory and `scripts/probe-lab`. Record run id,
profile, transport, session identity, public event ids, and relevant logs.
Keep the directory until the failure boundary is understood, then clean up
containers before the relay.

## Reporting uncertainty

Say:

- which claim passed;
- which claim failed;
- which claim was not exercised;
- whether the failure is product, harness, fixture, or live uncertainty;
- what evidence would distinguish the remaining possibilities.

Never report a compiled target, skipped scenario, or unexecuted live check as
passing behavior.

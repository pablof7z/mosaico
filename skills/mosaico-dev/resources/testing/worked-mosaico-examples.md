# Worked Mosaico examples

These examples show how complementary layers answer different questions.

## Explicit session anchor

Product claim:

> An explicitly selected sender session overrides ambient process hints.

BDD witness:

- launch two isolated sessions;
- send with the second session explicitly selected;
- query relay output and verify the message author is that session.

Lower-level evidence:

- CLI argument tests prove parsing;
- selection unit tests cover precedence combinations.

Why both: parser and precedence tests localize the rule; relay authorship proves
the complete supported behavior.

## Native Claude profile

Product claim:

> Agent `reviewer` using bundle `yolo-claude` launches Claude with bundle
> arguments followed by `--agent reviewer`, and no other harness selector.

BDD witness:

- real Mosaico binary and daemon;
- deterministic Claude shim;
- exact captured argv;
- assertion that no legacy terminal multiplexer ran.

Lower-level evidence:

- harness config parsing;
- profile-to-selector mapping;
- launch argument composition edge cases.

Do not use a real Claude model for deterministic argv proof.

## Cross-backend workspace discovery

Product claim:

> A workspace opened on backend `laptop` becomes visible to isolated backend
> `server` through the shared relay.

BDD witness:

- fresh Croissant relay;
- two homes with no shared filesystem state;
- relay metadata queried independently;
- `server` public workspace listing.

Lower-level evidence:

- NIP-29 event codec/contract tests;
- materializer/store tests;
- relay client acquisition integration.

Inspecting `laptop`'s database cannot prove cross-backend discovery.

## One daemon owns backend state

Product claim:

> A normal configured command starts one daemon that owns the backend socket.

BDD witness:

- run the exact binary in an isolated configured home;
- observe successful command and one owned socket.

Integration evidence:

- simultaneous client spawn race;
- stale socket reclaim;
- version-skew handshake;
- supported durable-store integrity checks.

BDD describes the stable operator outcome. Integration owns the race mechanics.

## Backend-addressed management command

Product claim:

> A backend-addressed `list sessions` command produces one public management
> result.

BDD witness:

- live deterministic harness session on local Croissant;
- operator-authored management message;
- relay reply containing the management result.

Lower-level evidence:

- management command parser cases;
- handler routing and rejected-command tests;
- reply event contract.

Do not write one Gherkin scenario for every parser token.

## Public relay NIP-29 behavior

Unknown:

> Does the deployed NIP-29 relay enforce closed/public membership and preserve
> readable group state?

Probe witness:

- explicit public relay;
- disposable identities and group;
- create, edit, add, write, and readback evidence.

Deterministic follow-up:

- pinned Croissant tests encode the learned group lifecycle;
- BDD proves Mosaico's product behavior against that fixture.

The public probe confirms deployment behavior at a moment; it does not replace
local regression.

## Addressed work disappears after agent add

Product failure:

> Work sent immediately after adding an agent can vanish without an explainable
> durable state.

BDD:

- `@must-never @wip @issue-291`;
- product outcome requires accepted, pending, delivered, or failed evidence;
- event explanation must say why delivery did or did not occur.

Causal regressions:

- add/session readiness transition;
- inbox claim and delivery state machine;
- relay materialization and harness delivery races.

The issue-linked excluded scenario is not green coverage. Once fixed, remove
`@wip` and run it deterministically.

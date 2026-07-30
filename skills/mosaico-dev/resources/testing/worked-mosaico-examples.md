# Worked Mosaico examples

These examples show how the evidence families answer different uncertainties.

## Explicit session authority

Claim:

> An explicitly selected sender session overrides ambient process hints.

Evidence:

- unit tests cover parsing and precedence combinations;
- an admitted black-box contract launches isolated sessions, sends through the
  exact binary, and verifies relay authorship.

Why both: the unit tests localize the rule; the relay oracle proves authority
across the supported path.

## Native profile selectors

Claim:

> Every harness adapter translates a named profile into its own valid selector
> while preserving shared Mosaico launch semantics.

Evidence:

- one typed adapter conformance suite covers Claude, Codex, Hermes, and other
  supported drivers;
- deterministic process captures cover executable argv boundaries where
  necessary;
- provider-specific cases cover only genuine differences.

This is not Cucumber. It is an adapter matrix, and no real model is needed.

## Cross-backend workspace discovery

Claim:

> A workspace opened on backend `laptop` becomes visible to isolated backend
> `server` through the shared relay.

Evidence:

- a small Cucumber contract uses fresh Croissant, isolated homes, independent
  relay metadata, and `server`'s public listing;
- codec, materializer, and acquisition tests localize lower-level failures.

Inspecting `laptop`'s database cannot prove cross-backend discovery.

## Message delivery across restart

Claim:

> Addressed work survives restart, remains bound to one public identity, and is
> delivered no more than once.

Evidence:

- unit/state-machine tests prove claim and delivery transitions;
- process tests prove durable restart mechanics;
- seeded schedule tests vary death before/after writes and acknowledgements;
- one black-box contract proves the stable outward identity/delivery promise.

Do not add one Gherkin scenario for each restart interleaving.

## Backend-addressed management

Claim:

> A backend-addressed management request produces one relay-visible result.

Evidence:

- parser and handler tests cover command cases;
- event contract tests cover authorship and reply relationships;
- an admitted Cucumber scenario proves the end-to-end relay-visible outcome.

The Cucumber scenario does not enumerate parser tokens.

## Public relay behavior

Question:

> Does the deployed NIP-29 relay currently enforce the group lifecycle Mosaico
> expects?

Evidence:

- an explicit live probe uses disposable identities and records side effects;
- stable findings are encoded against pinned Croissant;
- product contracts use the deterministic fixture, not the public relay.

The probe confirms compatibility at one time. It is not regression evidence.

## Agents notice overlapping work

Capability claim:

> Mosaico helps agents discover relevant peer work and reduces duplicate
> implementation without harming final task quality.

Evaluation:

- controlled repositories and paired assignments;
- no-Mosaico, awareness-only, and awareness-plus-messaging conditions;
- repeated runs across relevant model/harness combinations;
- task quality, time to useful contact, message relevance, duplicate diffs,
  conflicts, latency, and cost;
- independent scoring with saved transcripts and artifacts.

A valid run may use several different trajectories. One Gherkin path cannot
prove this capability.

## Feature removal

When a command or behavior is deleted, delete its scenarios, tests, fixtures,
and dedicated helpers. Do not add a test that invokes the old command and
expects rejection. Current replacement behavior receives its own positively
stated evidence.

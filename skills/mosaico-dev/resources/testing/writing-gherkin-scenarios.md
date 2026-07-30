# Writing admitted Gherkin scenarios

Write Gherkin only after a claim passes the admission rule in [BDD and
deterministic behavior contracts](bdd.md).

## Shape

- `Feature`: one coherent load-bearing product capability or invariant.
- `Scenario`: one concrete example that distinguishes a rule.
- `Given`: relevant externally meaningful starting state.
- `When`: the causal operator, agent, relay, or lifecycle event.
- `Then`: public or independent consequences.

Prefer one causal `When`. Do not turn scenarios into operator scripts.

## Vocabulary

Use current Mosaico product nouns:

- operator, agent, backend, session, workspace, channel;
- addressed work, membership, public identity;
- native harness, relay, diagnostic, visible result.

Avoid Rust functions, SQLite tables, helper names, private RPC handlers,
retired vocabulary, and implementation layout. Protocol kinds may appear only
when the wire contract itself is public and meaningful.

## Admission review

Before adding a file or scenario, record:

1. the product promise;
2. why a Rust contract alone lacks authority or durable clarity;
3. the deterministic fixture boundary;
4. the public/independent oracle;
5. the expensive failure it prevents;
6. the exclusive step glue it introduces.

Reject the scenario if it is an adapter matrix, schedule permutation,
capability evaluation, live check, issue plan, known failure, or tombstone.

## Example

```gherkin
@croissant
Scenario: A workspace opened on one backend appears on another
  Given a fresh NIP-29 relay
  And backends "laptop" and "server" have isolated homes
  And both backends trust the same operator
  When "laptop" starts an agent in workspace "mosaico"
  Then the relay holds the root channel for "mosaico"
  When "server" lists every visible workspace
  Then "server" shows workspace "mosaico"
  And no filesystem state is shared between the backends
```

The second `When` is justified because relay publication becomes the explicit
state observed by another isolated product boundary.

## Negative contracts

Use `@must-never` only for current, high-cost safety outcomes:

- peer input cannot gain host authority;
- work cannot silently lose all durable state;
- secrets cannot render publicly;
- hooks cannot start backend infrastructure.

Do not use negative scenarios to memorialize removed features, aliases, or
implementations.

## Scenario outlines

An adapter or provider matrix belongs in a typed conformance suite, not a
Gherkin outline. Use an outline only when several product-relevant examples of
the same outward rule remain readable and each row earns acceptance-level
evidence.

## Step vocabulary

Steps express domain actions:

- good: `the operator addresses that agent with "review this"`
- weak: `I run command "mosaico channel send ..."`
- forbidden: `the test calls operator_kind9_to_offline`

Keep vocabulary small and closed. Reuse a step only when its domain meaning is
identical. Remove exclusive steps when their last admitted scenario leaves.

## Committed-state rule

Every committed scenario runs in the deterministic suite. Do not commit
`@designed`, `@wip`, `@live`, issue-tracking, or historical-migration tags.
GitHub owns plans; live labs own external compatibility; the feature tree owns
only executable current contracts.

# Writing Gherkin scenarios

Feature prose is a durable product contract. An agent should be able to review
it without knowing Mosaico's Rust module layout.

## Shape

Use:

- `Feature` for one coherent product capability or invariant;
- `Scenario` for one concrete example of a rule;
- `Given` for relevant starting state;
- `When` for the event or decision under examination;
- `Then` for observable consequences;
- `And` only when it continues the same semantic role.

Prefer one causal `When`. A lifecycle scenario may contain a second `When` when
the intermediate result becomes the explicit state for the next action, but do
not turn a scenario into an operator script.

## Vocabulary

Use Mosaico product nouns:

- operator, agent, backend, session, workspace, channel;
- message, addressed work, membership, public identity;
- native harness, relay, diagnostic, visible result.

Avoid implementation nouns unless that surface is itself the public contract:

- Rust function and module names;
- SQLite tables and columns;
- private daemon handlers;
- helper filenames;
- internal task or channel types.

Protocol kinds may appear when relay protocol behavior is the contract. Prefer
“root channel metadata” over `kind:39000` when the number adds no meaning.

## Good scenario test

A scenario is ready when:

1. its title states the distinguishing outcome;
2. every `Given` affects the rule;
3. the `When` is an operator, agent, or external event;
4. each `Then` is visible through a supported or independent witness;
5. it remains meaningful if implementation modules are rewritten;
6. it runs independently from every other scenario;
7. it contains no secret material.

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

The independent relay witness and isolated filesystems distinguish fabric
discovery from accidental local sharing.

## Negative and must-never examples

Use negative scenarios for high-cost boundaries, not every rejected input.
Good candidates include:

- hooks never start backend infrastructure;
- peer text never gains host authority;
- addressed work never silently disappears;
- secrets never render publicly;
- path resolution never mints a phantom channel.

Put parser rejection permutations in unit or contract tests. Keep one BDD
example when the rejection itself is a stable product promise.

## Scenario outlines

Use `Scenario Outline` when every row demonstrates the same rule and the
differences are product-relevant. Native profile selectors across Claude,
Codex, and Hermes can form one rule if each row expects the same lifecycle
contract.

Do not use an outline to generate a cross-product of transports, errors, flags,
and internal states. Cover the rule's equivalence classes below the acceptance
layer.

## Step vocabulary

Steps should express reusable domain actions, not generic automation:

- good: `the operator addresses that agent with "review this"`
- weak: `I run a channel-send command`
- forbidden: `the test calls operator_kind9_to_offline`

Keep the vocabulary closed and intentional. Reuse a step when its domain
meaning is identical. Do not force reuse by adding ambiguous parameters or
branching behavior to one step.

Step definitions may perform complex setup, but the world must own all created
state and processes. Assertions should expose the actual witness in failure
output.

## Tag discipline

Apply tags to communicate truth and fixture needs, not organization alone:

- `@croissant` for real local NIP-29 semantics;
- `@must-never` for a deterministic safety invariant;
- `@live` for real credentials/public infrastructure;
- `@designed @issue-N` for agreed unimplemented behavior;
- `@wip @issue-N` for a known failing built behavior.

Never use `@wip` as a temporary convenience while developing a scenario. Run
the focused scenario locally. Commit an exclusion only for a real open bug with
a behavior-specific issue.

## Common rewrites

- “Then the sessions table has one row” becomes “Then the agent is live under
  the same public identity with no sibling.”
- “When the management handler parses list” becomes “When the operator sends
  management command `list sessions`.”
- “Then spawn_args equals the vector” becomes “Then the Claude process receives
  exactly the bundle arguments and profile selector.”
- “Then the test succeeds” becomes the public or independent fact that defines
  success.

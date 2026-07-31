# Probes, seeds, and live labs

These checks use infrastructure that deterministic regression suites cannot
fully control. They answer different questions and must remain explicit.

## Probe

A probe asks a bounded question about an external authority. For example:

- does a public relay deliver an event p-tagged to identity B over a connection
  authenticated as identity A?
- does the configured NIP-29 relay honor a client-chosen group id?
- can a non-member read a closed, public group?

Current ignored targets include `tests/relay_probe.rs` and
`tests/nip29_probe.rs`.

A probe must declare:

- the exact question and why local code cannot answer it;
- required environment variables;
- published state or other side effects;
- timeout and rate-limit behavior;
- evidence printed;
- what deterministic test or architecture decision consumes the finding.

A successful probe is evidence about that infrastructure at that time. It is
not a routine regression guarantee.

## Validation seed

`tests/seed_validation.rs` publishes a self-contained Mosaico session for a
reader application to inspect. It uses the Rust test harness, but its purpose
is fixture creation and readback validation. It leaves public state.

Do not call a seed a test merely because `cargo test` launches it. Name the
side effect, use disposable identities and channels, and require an explicit
relay environment variable.

## Live provider lab

The `mosaico-dev` skill's live-lab workflow runs real provider auth and transport through isolated
container profiles. Use it to prove:

- host authentication is staged correctly;
- PTY, ACP, or app-server startup succeeds;
- an admitted session receives fabric context or addressed input;
- native session resume works across a provider-process restart;
- hooks/plugins are installed at the provider's real locations.

The objective is transport and fabric proof, not model quality. Use the
cheapest model capable of one deterministic instruction. Do not assert that a
provider produced a “good” review or plan.

Start with `references/lab/INDEX.md`, then use the provider-specific
references and `scripts/probe-lab`.

## Deterministic versus live

Keep deterministic CI free of:

- provider credentials;
- public relay availability;
- rate limits;
- model wording;
- mutable provider versions;
- host-global Mosaico state.

When a live probe establishes a stable rule, reproduce it with a pinned local
fixture where possible. Croissant-backed BDD is the deterministic home for
NIP-29 product contracts; public-relay probes remain confirmation of deployed
behavior.

## Separation from deterministic contracts

Live providers and public infrastructure never run through the Cucumber
contract suite. The same product invariant may have deterministic local
evidence, while this guide owns current external compatibility and operational
proof.

Model-dependent coordination belongs in
[agent capability evaluations](agent-capability-evaluations.md), not a live
pass/fail feature scenario.

Run current relay probes with:

```sh
MOSAICO_RELAY=wss://relay.example just test-live-relay-probe
MOSAICO_NIP29_RELAY=wss://relay.example just test-live-nip29-probe
```

The configured URLs must be intentional. Never default a destructive probe to
production infrastructure.

## Reporting

Record relay URL, run id, generated profile names, exact accepted commands,
transport/bundle, session identity, event ids, provider auth result, relevant
logs, and cleanup status. Never include private keys or provider tokens.

If a live check fails, preserve its work directory until the boundary is
understood. Do not turn a variable live failure into a blind retry loop.

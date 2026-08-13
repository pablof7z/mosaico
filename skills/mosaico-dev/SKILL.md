---
name: mosaico-dev
description: "Develop Mosaico itself. Load for any work in this repo. Not for fabric participation (use mosaico)."
---

# Mosaico development

Before changing product behavior, read the **repo root `AGENTS.md`**. It owns
backwards-compat policy, file-size limits, the GitHub Issues backlog rule, and
daemon restart safety. Product intent lives under `docs/product-spec/`;
architecture under `docs/fabric-architecture*.md` and the other design docs in
`docs/`. Prefer those over memory or ad-hoc plan files.

Expand: `skills/mosaico-dev/resources/authority-and-orientation.md`.

## Requested behavior is active

Ship requested product behavior on the normal runtime path. Do not hide it
behind an environment variable, config boolean, rollout toggle, experimental
switch, or undocumented opt-in unless the user or settled product design
explicitly requires staged or genuinely optional behavior. This rule is about
runtime activation gates, not Cargo features or ordinary configuration that
selects required resources such as relays or providers.

Expand: `skills/mosaico-dev/resources/authority-and-orientation.md`.

## Launch and config

Care when you touch how sessions start or how agents/harnesses are declared.
The durable contract is: bundles own transport; agents pick a bundle and
identity mode; launch is `mosaico <TARGET> [PROMPT] [-- <ARGS>...]` with no
launch-time harness/transport switch. Identity keys have fixed roles — do not
invent dual names or legacy flags.

Expand: `skills/mosaico-dev/resources/launch-and-config.md`.

## Build, test, and quality

Default development loop for almost every code change. Run the repo `just`
recipes (fmt, lint, LOC, unit, hermetic, local relay/contracts) instead of
inventing cargo one-offs. Croissant is an external binary Mosaico does not own.
Write the claim/oracle before implementing; do not quietly weaken tests to
match code.

Expand: `skills/mosaico-dev/resources/build-and-quality.md` and
`skills/mosaico-dev/resources/testing/INDEX.md`.

## Containers and live lab

**Why it exists:** unit and hermetic tests cannot prove real provider auth,
PTY/ACP wiring, host-auth staging, or end-to-end fabric delivery through Claude,
Codex, Grok, Goose, Hermes, Kimi, or OpenCode. The live lab is the opt-in stack for
that class of proof: host Croissant relay + isolated container profiles + real
host credentials.

**When to use it:** the change or investigation depends on a real provider CLI,
hosted session lifecycle, hooks/plugins at provider paths, multi-agent or
multi-human traffic on a real relay, or you are validating install/onboarding
in the container runner. **When not to:** pure logic, schema, deterministic
contracts, or anything a `just` suite already covers — stay hermetic.

**What to care about:** keep fabric state in the lab profile, not host
`~/.mosaico`; never attach a second container to a live profile (socket
eviction); inspect from the host while an agent is up; clean containers before
the relay. Objective is transport/fabric evidence, not model quality — use the
cheapest model that can run one command.

Expand: `skills/mosaico-dev/resources/containers-and-lab.md` and the procedure
index `skills/mosaico-dev/references/lab/INDEX.md`.

## Where detail lives

| Path under `skills/mosaico-dev/` | Open when you need… |
|---|---|
| `resources/authority-and-orientation.md` | sources of truth, working rules, repo layout |
| `resources/launch-and-config.md` | harnesses.json, agents, identity, launch CLI |
| `resources/build-and-quality.md` | just recipes, Croissant, quality gates |
| `resources/testing/` | how to choose and write tests |
| `resources/containers-and-lab.md` | lab purpose, hard rules, minimal start |
| `references/lab/` | step-by-step live lab procedure |
| `references/` | backends, ACP, Grok, observability, troubleshooting |
| `scripts/` | relay, profile write, launch, probe, cleanup helpers |
| `tests/` | checks for those scripts |

---
name: mosaico-dev
description: "Develop Mosaico itself. Load for any work in this repo. Not for fabric participation (use mosaico)."
---

# Mosaico development

Operator guide for **building** Mosaico. The sibling **`mosaico`** skill is for
*participating* in a running fabric; this skill is for implementing, testing,
and operating the product that provides it.

## Authority and orientation

[`AGENTS.md`](../../../AGENTS.md) is the contributor contract (no backwards
compat, file size, GitHub Issues as the only queue, daemon restart rules).
Product and architecture live under `docs/`. Read the owning doc before inventing
behavior.

Details: [`resources/authority-and-orientation.md`](resources/authority-and-orientation.md).

## Launch and config

Harness bundles and agent files own transport and identity. Launch is
`mosaico <TARGET> [PROMPT] [-- <ARGS>...]` — no launch-time harness/transport
selector. Keep `userNsec` and `mosaicoPrivateKey` distinct; per-session agents
are keyless on disk.

Details: [`resources/launch-and-config.md`](resources/launch-and-config.md).

## Build, test, and quality

Use `just` recipes for fmt, lint, LOC, unit, hermetic, and relay-backed suites.
Croissant is external. Admit test oracles before implementation; do not weaken
them around code.

Details: [`resources/build-and-quality.md`](resources/build-and-quality.md),
[`resources/testing/INDEX.md`](resources/testing/INDEX.md).

## Containers and live lab

Isolated container profiles prove host auth and transport. Never start a second
container against a live profile; inspect from the host. Clean containers before
the relay.

Details: [`resources/containers-and-lab.md`](resources/containers-and-lab.md),
[`references/lab/INDEX.md`](references/lab/INDEX.md).

## Resource map

| Path | Role |
|---|---|
| [`resources/authority-and-orientation.md`](resources/authority-and-orientation.md) | Sources of truth, how to work, repo map |
| [`resources/launch-and-config.md`](resources/launch-and-config.md) | Bundles, agents, launch CLI, identity |
| [`resources/build-and-quality.md`](resources/build-and-quality.md) | Just recipes, Croissant, contracts |
| [`resources/testing/`](resources/testing/INDEX.md) | Full testing doctrine and commands |
| [`resources/containers-and-lab.md`](resources/containers-and-lab.md) | Container/lab rules and quick start |
| [`references/lab/`](references/lab/INDEX.md) | Live lab procedure topics |
| [`references/`](references/lab/INDEX.md) | Backend, ACP, Grok, observability, troubleshooting |
| `scripts/` | Relay, profiles, launch, probe, cleanup helpers |
| `tests/` | Skill script tests (`scripts.sh`, `profile-writer.sh`) |

---
name: mosaico-dev
description: "Develop Mosaico itself: product code, tests, docs, containers, and live labs. Load for any work in this repo. Not for fabric participation (use mosaico)."
---

# Mosaico development

This skill is the operator guide for **building Mosaico**.

The sibling **`mosaico`** skill is different: it teaches an agent how to
*participate* in a running fabric. This skill teaches how to *implement, test,
and run* the system that provides that fabric.

## Authority

| Concern | Source of truth |
|---|---|
| Contributor rules (compat, LOC, planning, daemon restart) | repo root [`AGENTS.md`](../../../AGENTS.md) |
| Product doctrine | `docs/product-spec/` |
| Architecture | `docs/fabric-architecture.md`, `docs/fabric-architecture-overview.md`, design docs under `docs/` |
| Source-backed synthesis | `docs/wiki/` when present |
| Testing doctrine and commands | [`resources/testing/INDEX.md`](resources/testing/INDEX.md) |
| Container runner | `containers/mosaico/README.md` and `containers/mosaico/run` |
| Tactical backlog | open GitHub issues only — no parallel plan files |

`AGENTS.md` is enforced, not suggested. Do not restate its rules elsewhere as a
second queue; correct durable docs in place when they drift.

## How to work

1. **Orient from authority.** Read the owning doc or module before inventing
   behavior. Prefer `docs/` and `AGENTS.md` over chat memory or stale plans.
2. **No backwards compatibility.** Remove dead surfaces completely in the same
   change. No aliases, legacy flags, fallback JSON keys, or dual names.
3. **One tactical queue.** Open or update a GitHub issue; do not create
   `TODO.md` / `PLAN-*.md` / scattered roadmaps. Retire executed plans.
4. **File size.** Soft 300 LOC, hard 500 LOC for hand-authored source. Split on
   domain boundaries; keep extracted visibility narrow.
5. **Daemon safety.** Never kill live PTY supervisors by bare binary name.
   Restart only the daemon process (`pkill -f 'mosaico daemon'`); see
   `AGENTS.md`.
6. **Secrets.** Never print provider credentials, Nostr secrets, `userNsec`,
   `mosaicoPrivateKey`, or agent private keys.
7. **Prove the right layer.** Unit/contract for pure rules; hermetic or local
   relay for process boundaries; live lab only for real-provider transport and
   auth. See the testing index.

## Orientation map

```text
AGENTS.md                 contributor contract
docs/product-spec/        why and product shape
docs/fabric-architecture* how the fabric works
docs/harness-integration  provider/harness boundary
docs/daemon-*.md          daemon RPC and lifecycle
containers/mosaico/       isolated image + runner
skills/mosaico/           agent-facing fabric skill (shipped to users)
skills/mosaico-dev/       this skill (developer tooling)
e2e/                      black-box and behavior-contract surfaces
```

When unsure where a concept lives, search the repo and correct the owning doc —
do not invent a parallel note.

## Launch and config contracts

These ownership boundaries are product law for config and CLI work:

- `harnesses.json` maps a bundle name to exactly `harness`, `transport`, and
  optional `args`. Unknown fields fail parsing. The executable and transport
  driver are code-owned.
- `agents/<slug>.json` owns the public slug, selected bundle in `harness`,
  optional harness-native `profile`, identity mode, and metadata.
- `mosaico <TARGET> [PROMPT] [-- <ARGS>...]` matches an existing session, then
  an available agent. Workspace is the current directory; accepts `--channel`
  and `--name`. Args after `--` append to the resolved harness command for that
  launch only.
- A bundle admits exactly one hosted transport: `pty` or `acp`. A configured
  `app-server` bundle uses the ACP hosted kind with the app-server dialect;
  `app-server` is not a third admitted kind. There is no launch-time transport
  or harness selector.
- Bundle `args` are operational provider flags. Agent `profile` is a named
  native profile (Claude PTY `--agent`, Codex PTY `--profile`, Hermes
  PTY/ACP top-level `--profile`, Codex app-server isolated `CODEX_HOME`). ACP
  dialects without named profiles reject `profile`.

Never add old launch flags, duplicate config fields, or fallback bundle names.
Fix durable defaults in config; use separator args only for intentional
one-launch overrides.

### Identity

- `userNsec` is the human operator signer. `mosaicoPrivateKey` is the backend
  management/session-derivation identity. They must be distinct.
- `perSessionKey: true` agents are keyless on disk (omit `secret_key` /
  `public_key`); session keys derive from the backend key plus a fresh anchor.
- `perSessionKey: false` requires persisted agent `secret_key` and `public_key`.

### Harness notes (when touching integrations)

- **Grok:** native hooks install at `.grok/hooks/mosaico.json`. Imported Claude
  hooks are not Grok proof.
- **Goose:** Mosaico Open Plugin + Top Of Mind refresh for both `goose session`
  and `goose acp`. Goose ACP has no stable recipe/profile selector.
- **Hermes:** isolated `HERMES_HOME` with Mosaico user plugin and named profiles.

## Build, test, and quality

Prefer the repo's `just` recipes over ad-hoc cargo invocations when a recipe
exists. Details and suite ownership live in
[`resources/testing/INDEX.md`](resources/testing/INDEX.md) and
[`resources/testing/ci-and-local-commands.md`](resources/testing/ci-and-local-commands.md).

Common entry points:

```bash
just fmt-check
just lint
just loc-check
just test-unit
just test-hermetic-integration
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-local-nip29
NIP29_RELAY_BIN=/absolute/path/to/croissant just test-behavior-contracts
NIP29_RELAY_BIN=/absolute/path/to/croissant just test   # full local aggregate
```

Croissant is an **external** binary. Mosaico never builds or owns it. Resolve
via `MOSAICO_DEV_CROISSANT_BIN`, `NIP29_RELAY_BIN`, or `croissant` on PATH.

Behavior-contract discipline: admit the claim and oracle before implementation;
do not silently weaken tests around code. See the testing index.

## Containers and live lab

Use the container runner for isolated host-auth backends and transport proof.
Lab procedure: [`references/lab/INDEX.md`](references/lab/INDEX.md).

**Non-negotiables for lab / container work:**

- Real host AI auth (`MOSAICO_CONTAINER_HOST_AUTH=1` default).
- Fabric state under `.container-state/<profile>` or the run workdir — never
  host `~/.mosaico`.
- Cheapest model that can run one command and report a result.
- `direct` = provider auth/plugin only. `launch` = hosted lifecycle. Run
  `__acp-smoke` before structured ACP/app-server launch.
- **Never** start a second container against a profile whose agent is alive
  (shared socket eviction). Inspect bind-mounted logs and the relay from the
  host only while live.
- Clean containers before the relay: `scripts/cleanup-lab`.

**Minimal lab start** (from repo root):

```bash
export MOSAICO_DEV_CROISSANT_BIN="${MOSAICO_DEV_CROISSANT_BIN:-$(command -v croissant)}"
bash containers/mosaico/run build-image
bash containers/mosaico/run doctor
skills/mosaico-dev/scripts/start-croissant-relay
# keep printed LAB_ENV=...
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" <profiles...>
```

Manual first-time setup without pre-generated lab config:

```bash
bash containers/mosaico/run onboard
```

## Resource map

### Live lab and backends

- `references/lab/` — live lab procedure (`INDEX.md`, start, prewarm, launch,
  traffic, inspect-and-cleanup)
- `references/container-backends.md` — auth, state, identity, profiles
- `references/acp-backends.md` — ACP / app-server smoke and launch
- `references/grok-pty-lab.md` — native Grok hooks and delivery proof
- `references/observability.md` — safe evidence surfaces and report format
- `references/troubleshooting.md` — failures and cleanup
- `scripts/start-croissant-relay`, `write-container-profiles`, `launch-agent`,
  `probe-lab`, `cleanup-lab`, `send-human-kind9`

### Testing

- `resources/testing/INDEX.md` — full testing guide index
- `resources/testing/ci-and-local-commands.md` — exact recipes and CI shape
- `resources/testing/probes-seeds-and-live-labs.md` — live evidence family

### Tests for this skill's scripts

- `tests/scripts.sh`, `tests/profile-writer.sh`

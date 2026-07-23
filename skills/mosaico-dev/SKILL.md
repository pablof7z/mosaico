---
name: mosaico-dev
description: "Use for Mosaico development live labs: run a local croissant relay, configure isolated PTY or ACP hosted bundles (including the app-server ACP dialect) with real host AI auth, launch Claude/Codex/Grok/Goose/Hermes/OpenCode agents, and inspect sessions, logs, relay traffic, and Nostr events."
---

# Mosaico development live lab

Use this skill to validate Mosaico on a real local stack: a host croissant
relay, isolated container backends, real provider auth, Mosaico hooks, and
inspectable relay and daemon evidence. The objective is transport and fabric
proof, not model quality.

## Resource map

- `references/live-lab-workflow.md`: start-to-finish single- and multi-agent
  procedure.
- `references/container-backends.md`: auth, state, identity, profile, and model
  boundaries.
- `references/grok-pty-lab.md`: native Grok hooks, p-tagged PTY injection,
  provenance, delivery state, and reply proof.
- `references/acp-backends.md`: ACP/app-server configuration, smoke, and launch.
- `references/observability.md`: safe evidence surfaces and report format.
- `references/troubleshooting.md`: concrete failure checks and cleanup.
- `scripts/start-croissant-relay`: starts a host relay and writes `lab.env`.
- `scripts/write-container-profiles`: writes current device, harness-bundle, and
  agent-selection state.
- `scripts/launch-agent`: runs a provider directly, runs `__acp-smoke`, or calls
  the current direct Mosaico launch surface.
- `scripts/probe-lab`: captures relay metadata, logs, and Nostr events.
- `scripts/cleanup-lab`: stops recorded containers before stopping the relay.

## Current launch contract

Treat these ownership boundaries as fixed:

- `harnesses.json` maps a bundle name to exactly `harness`, `transport`, and
  optional `args`. Unknown fields fail parsing. The executable and transport
  driver are code-owned.
- `agents/<slug>.json` owns the public slug, selected bundle in `harness`,
  optional harness-native `profile`, identity mode, and metadata.
- `mosaico <TARGET> [PROMPT] [-- <ARGS>...]` first matches an existing session,
  then an available agent. The workspace comes from the current directory; it
  accepts `--channel` and `--name`. Arguments after `--` are appended to the
  resolved harness command for that launch.
- The selected bundle admits exactly one hosted transport kind: `pty` or `acp`.
  A configured `app-server` bundle uses the ACP hosted kind with the app-server
  protocol dialect; `app-server` is not a third admitted kind. There is no
  launch-time transport or harness selector.
- Bundle `args` are operational provider flags. An agent `profile` is a named
  native profile: Claude PTY applies `--agent`, Codex PTY applies `--profile`,
  Hermes PTY and ACP apply the top-level `--profile`, and Codex app-server
  composes the named config into an isolated `CODEX_HOME`. ACP dialects that do
  not support a named profile reject it.

Never add old launch flags, duplicate config fields, or fallback bundle names.
Fix durable defaults in generated config; use separator arguments only for an
intentional one-launch override.

## Identity contract

- `userNsec` is the human operator signer. `mosaicoPrivateKey` is the backend
  management/session-derivation identity. They must be distinct.
- `perSessionKey: true` agents are keyless on disk: omit `secret_key` and
  `public_key`. A fresh session derives its key from the backend key plus a
  fresh anchor.
- `perSessionKey: false` is the durable identity mode and requires a persisted
  agent `secret_key` and `public_key`.
- Never print provider credentials, Nostr secrets, or private-key fields.

## Non-negotiables

- Use real host AI auth. The container runner defaults to
  `MOSAICO_CONTAINER_HOST_AUTH=1` and stages writable provider state under the
  selected isolated profile.
- Grok auth is copied into writable isolated `GROK_HOME`; native Mosaico hooks
  install at `.grok/hooks/mosaico.json`. Imported Claude hooks are not Grok proof.
- Goose config and keychain secrets are copied into its isolated XDG home.
  Mosaico installs its Goose Open Plugin there; awaited lifecycle hooks refresh
  a session-specific Top Of Mind file before each model turn. Both interactive
  `goose session` and managed `goose acp` launches require this integration.
  Goose ACP exposes no stable recipe/profile selector, so do not advertise
  recipes as native profiles.
- Hermes config, environment, and named profiles are copied into isolated
  `HERMES_HOME`, where Mosaico installs its user plugin.
- Keep fabric state under `.container-state/<profile>` or the run's temporary
  work directory, never host `~/.mosaico`.
- Run an externally installed Croissant on the host.
  `start-croissant-relay` resolves `MOSAICO_DEV_CROISSANT_BIN`,
  `NIP29_RELAY_BIN`, or `croissant` on PATH; Mosaico never builds or owns it.
- Use the cheapest provider model sufficient to run one command and report a
  result.
- Use direct mode only for provider auth/plugin checks. Use launch mode for
  hosted lifecycle, PTY, transport routing, native-profile activation, and
  delivery checks. Use `__acp-smoke` before a structured launch.
- Never start a second container against a profile whose launched agent is
  alive. The second daemon can replace the shared socket and evict the active
  session. Inspect bind-mounted logs and the relay from the host instead.

## Standard start

From the repository root:

```bash
git status -sb
bash containers/mosaico/run build-image
bash containers/mosaico/run doctor
skills/mosaico-dev/scripts/start-croissant-relay
```

To provision several human operators in one lab, give each a distinct display
name. The first identity owns the relay; all identities are generated under the
run directory and whitelisted in every generated backend profile:

```bash
MOSAICO_DEV_HUMAN_NAMES_JSON='["Pablo","Alice","Bob"]' \
  skills/mosaico-dev/scripts/start-croissant-relay
```

If the runner reaps background descendants, set
`MOSAICO_DEV_RELAY_FOREGROUND=1` and clean up from another terminal.

Keep the emitted environment path:

```bash
LAB_ENV=/tmp/mosaico-live-lab-YYYYmmdd-HHMMSS/lab.env
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" \
  claude claude-acp codex codex-app-server grok goose goose-acp hermes hermes-acp \
  opencode opencode-acp
```

The writer resets disposable Mosaico state, including SQLite/WAL state and the
NMP `nmp.redb` store, while preserving provider home and build caches.

## Manual onboarding

To inspect the real first-time setup wizard without pre-generating live-lab
configuration or importing host provider credentials:

```bash
bash containers/mosaico/run onboard
```

This builds the current checkout inside the standard lab image and runs
`mosaico setup` in the isolated `onboarding` profile. The completed state stays
under `.container-state/onboarding`. Reset that exact profile before replaying:

```bash
bash containers/mosaico/run --profile onboarding clean-state
```

Do not use `write-container-profiles` for this workflow; it intentionally
bypasses first-time device configuration.

Prewarm the exact profile with a real supported operation:

```bash
bash containers/mosaico/run --profile claude-acp doctor
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

For a PTY profile, `doctor` performs the Cargo build and hook installation. A
small direct provider prompt can additionally prove auth before an interactive
launch.

## Launch patterns

Direct provider mode may receive provider arguments:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct claude -p \
  "Respond with exactly OK." --model haiku
```

Launch mode receives only a generated target and optional prompt:

```bash
bash containers/mosaico/run --profile claude mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session and summarize the self header." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude
```

Native Grok uses the same PTY shape:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct grok \
  -p "Respond with exactly OK."
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch grok
```

The same form launches structured profiles; the bundle transport owns the
choice:

```bash
bash containers/mosaico/run --profile claude-acp mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude-acp
```

For Goose, install the plugin, generate `goose-acp`, run doctor, and run smoke
before launch. The smoke must prove fabric context in the model response and
pass both ACP turns across a process restart using `session/load`.

To audit launch inventory, run `mosaico agents` without an action. In a
non-interactive command it prints the available configured agents, raw harness
targets, and installed native profiles. Codex discovery includes custom agents
plus named `$CODEX_HOME/*.config.toml` profiles that carry non-empty
`developer_instructions`; other harnesses use their global/workspace native
directories. If one slug exists in multiple harnesses, select the suffixed
target printed by the inventory; that selection persists the binding. An
existing binding keeps the bare slug while other harness implementations remain
available as suffixed rebinding choices.

## Safe inspection

While a launch container is alive, use host-only evidence:

```bash
skills/mosaico-dev/scripts/probe-lab "${LAB_ENV}"
tail -n 200 .container-state/claude-acp/mosaico/daemon.log
tail -n 200 .container-state/claude-acp/mosaico/relay.log
```

Do not run `containers/mosaico/run --profile <same-profile> ...` concurrently,
including a bare `mosaico` invocation, `channel`, `debug explain`, or `debug
hook-tail`. For a PTY run, use the terminal already attached by launch. After
stopping the launch container, the operator may use `mosaico` or other same-profile tools.

Send a real tagged mention from a safe sender profile or after the target is
stopped:

```bash
mosaico channel send --channel <channel> --tag <session-handle> \
  --message "Run mosaico my session."
```

For a generated human identity, publish its named kind:0 profile and signed
kind:9 directly from the host without exposing the signer:

```bash
skills/mosaico-dev/scripts/send-human-kind9 "${LAB_ENV}" Alice \
  <channel> <session-pubkey> "Identify this sender and reply to this event."
```

Literal `@handle` text is not a tag. Use `--force` only when the literal text is
intentional.

## Evidence standard

Report the relay URL and run id, generated profiles, exact accepted commands,
transport/bundle metadata, PTY or RPC session id, provider auth result,
croissant and `nak` evidence, relevant daemon/hook logs, and a feature-specific
pass/fail. If a step fails, include the first failing command and preserve its
work directory until the failure is understood.

Always clean up recorded containers before the relay:

```bash
skills/mosaico-dev/scripts/cleanup-lab "${LAB_ENV}"
```

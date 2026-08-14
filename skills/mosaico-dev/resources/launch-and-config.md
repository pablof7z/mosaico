# Launch and config contracts

**Why:** session start and agent declaration are product surface. Drift here
ships broken hosts and fake compatibility.

**When:** editing launch CLI, `harnesses.json`, agent JSON, identity derivation,
or harness-native profile wiring.

No old launch flags, duplicate config fields, or fallback bundle names. Fix
durable defaults in config; use separator args only for intentional one-launch
overrides.

## Bundle and agent files

- `harnesses.json` maps a bundle name to exactly `harness`, `transport`, and
  optional `args`. Unknown fields fail parsing. The executable and transport
  driver are code-owned.
- `agents/<slug>.json` owns the public slug, selected bundle in `harness`,
  optional harness-native `profile`, identity mode, and metadata.

## Launch surface

- `mosaico <TARGET> [PROMPT] [-- <ARGS>...]` matches an existing session, then
  an available agent. Workspace is the current directory; accepts `--channel`
  and `--name`. Args after `--` append to the resolved harness command for that
  launch only.
- A bundle admits exactly one hosted transport: `pty`, `acp`, `app-server`, or
  `pi-rpc`. A configured
  `app-server` bundle uses the ACP hosted kind with the app-server dialect;
  `app-server` is not a third admitted kind. There is no launch-time transport
  or harness selector.
- Bundle `args` are operational provider flags. Agent `profile` is a named
  native profile (Claude PTY `--agent`, Codex PTY `--profile`, Hermes
  PTY/ACP top-level `--profile`, Kimi PTY `--agent`, Codex app-server isolated
  `CODEX_HOME`). ACP dialects without named profiles reject `profile`.

## Identity

- `userNsec` is the human operator signer. `mosaicoPrivateKey` is the backend
  management/session-derivation identity. They must be distinct.
- `perSessionKey: true` agents are keyless on disk (omit `secret_key` /
  `public_key`); session keys derive from the backend key plus a fresh anchor.
- `perSessionKey: false` requires persisted agent `secret_key` and `public_key`.

## Harness notes

- **Grok:** native hooks install at `.grok/hooks/mosaico.json`. Imported Claude
  hooks are not Grok proof.
- **Goose:** Mosaico Open Plugin + Top Of Mind refresh for both `goose session`
  and `goose acp`. Goose ACP has no stable recipe/profile selector.
- **Hermes:** isolated `HERMES_HOME` with Mosaico user plugin and named profiles.
- **Kimi:** managed TOML hooks in `KIMI_CODE_HOME/config.toml`; PTY and native
  `kimi acp` transports are supported. PTY named agents use `kimi --agent`;
  Kimi ACP rejects profiles because it exposes no agent selector. Its `Stop`
  hook uses native block output to deliver pending fabric
  context before the model finishes and closes the original turn accounting.
- **Pi:** the global extension at `PI_CODING_AGENT_DIR/extensions/mosaico.ts`
  maps Pi lifecycle events to Mosaico hooks. PTY uses `pi` and `--session`;
  managed mode uses `pi --mode rpc`, persists `pi-rpc`, and completes only on
  `agent_end`. Pi exposes no named-profile selector, so both transports reject
  `profile`.

Provider-specific lab detail:
`skills/mosaico-dev/references/container-backends.md`,
`skills/mosaico-dev/references/acp-backends.md`,
`skills/mosaico-dev/references/grok-pty-lab.md`.

# Launch and config contracts

**Why:** session start and agent declaration are product surface. Drift here
ships broken hosts and fake compatibility.

**When:** editing launch CLI, `presets.json`, agent JSON, identity derivation,
transport selection, or harness-native profile wiring.

There is no backwards compatibility for removed launch configuration. Reject
old names and fields; do not add aliases, fallback reads, or conversions.

## Agent and preset files

- `agents/<slug>.json` owns the public slug, canonical `harness`, optional
  harness-native `profile`, optional named `preset`, identity mode, and metadata.
- `presets.json` is preset-first: preset name, canonical harness, then optional
  `pty`, `acp`, and `app-server` string arrays. Unknown fields fail parsing.
- A preset only adds arguments to a driver selected elsewhere. It never chooses
  a harness, executable, or transport. No preset means no implicit arguments.

```json
{
  "unrestricted": {
    "codex": {"pty": ["--yolo"]},
    "claude-code": {"pty": ["--dangerously-skip-permissions"]}
  }
}
```

## Launch surface

- `mosaico <TARGET> [PROMPT] [-- <ARGS>...]` resolves a session, then an agent.
  Args after `--` append last for that launch only.
- Interactive fresh launch selects PTY. Managed launch selects the preferred
  structured driver when supported: Codex app-server or native ACP.
- Resume preserves an admitted managed transport when it remains supported and
  re-resolves the agent's current preset for the new runtime generation.
- `profile` activates a harness-native named profile. Unsupported combinations
  fail instead of silently dropping it.

## Identity

- `userNsec` is the human operator signer. `mosaicoPrivateKey` is the backend
  management/session-derivation identity. They must be distinct.
- `perSessionKey: true` agents are keyless on disk.
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

Provider-specific lab detail lives in `references/container-backends.md`,
`references/acp-backends.md`, and `references/grok-pty-lab.md`.

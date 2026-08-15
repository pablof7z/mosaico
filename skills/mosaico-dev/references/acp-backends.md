# ACP and app-server backends

Use this reference for Claude ACP, Codex app-server, Goose ACP, Hermes ACP,
Kimi ACP, OpenCode ACP, and Pi RPC labs.
These transports use structured RPC instead of terminal-byte injection.

## Configuration contract

A configured agent selects a canonical harness and optional preset:

```json
{
  "slug": "claude",
  "created_at": 0,
  "perSessionKey": true,
  "harness": "claude-code",
  "preset": "lab"
}
```

The preset can add arguments for the transport selected by managed launch:

```json
{
  "lab": {
    "claude-code": {"acp": []},
    "codex": {"app-server": ["-c", "model=test"]}
  }
}
```

`presets.json` does not select transport. A missing transport cell contributes
no arguments. A referenced preset or harness realization that does not exist is
an error. Per-session agents are intentionally keyless on disk.

## Generated profiles

| profile | harness | managed transport | args override |
| --- | --- | --- | --- |
| `claude-acp` | `claude-code` | `acp` | `MOSAICO_DEV_CLAUDE_ACP_ARGS_JSON` |
| `codex-app-server` | `codex` | `app-server` | `MOSAICO_DEV_CODEX_APP_SERVER_ARGS_JSON` |
| `goose-acp` | `goose` | `acp` | `MOSAICO_DEV_GOOSE_ACP_ARGS_JSON` |
| `opencode-acp` | `opencode` | `acp` | `MOSAICO_DEV_OPENCODE_ACP_ARGS_JSON` |
| `hermes-acp` | `hermes` | `acp` | `MOSAICO_DEV_HERMES_ACP_ARGS_JSON` |
| `kimi-acp` | `kimi` | `acp` | `MOSAICO_DEV_KIMI_ACP_ARGS_JSON` |
| `pi-rpc` | `pi` | `pi-rpc` | `MOSAICO_DEV_PI_RPC_ARGS_JSON` |

Default args are `[]`. The override must be a JSON string array. Native profile
support is driver-specific: Hermes ACP supports `--profile`; Codex app-server
uses isolated `CODEX_HOME` composition; Claude, Goose, OpenCode, and Kimi ACP
do not expose a supported named-profile selector.

## Smoke before launch

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" claude-acp
bash containers/mosaico/run --profile claude-acp doctor
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

The smoke command passes the canonical harness plus the generated named preset
to `mosaico __acp-smoke`. ACP proves `session/load`; Codex app-server proves
`thread/resume` in a fresh process.

Inspect public configuration without printing signer or provider secrets:

```bash
jq . .container-state/claude-acp/mosaico/presets.json
jq '{slug,harness,preset,profile,perSessionKey,has_secret:has("secret_key")}' \
  .container-state/claude-acp/mosaico/agents/claude.json
```

The smoke prints the resolved argv, first session ID, successful cross-process
`session/load`, both `end_turn` results, and `PASS`.

Kimi's canonical structured command is `kimi acp`. The same smoke contract
proves initialization, two real turns, and cross-process `session/load`.

Pi's canonical structured command is `pi --mode rpc`. Its protocol is strict
line-delimited JSON rather than ACP: `get_state` provides the native session
ID, `prompt` acceptance starts a turn, `agent_end` completes it, and a new
process resumes with `--session <id>`. Pi exposes no named-profile selector.

## Launch

Register the workspace and supply an optional positional prompt through the
helper environment:

```bash
bash containers/mosaico/run --profile claude-acp mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude-acp
```

The helper calls the current `mosaico <slug> [prompt] [-- <args>...]` form. Any
trailing helper arguments are forwarded after the separator for that launch.
The selected bundle transport causes the helper to keep the container alive
after the launch command returns.

Expected output includes `[mosaico acp] session: ...`. There is no PTY to
attach. While the container is alive, inspect bind-mounted logs and host-side
relay probes only. Do not start another container against the same profile.

## Troubleshooting

If resolution fails, compare the agent's `harness` string to the exact bundle
key and validate that each bundle has only `harness`, `transport`, and optional
`args`. Do not add alternate filenames, duplicate fields, fallback commands, or
launch-time selectors.

If Claude asks to install the adapter, rebuild the image and rerun doctor:

```bash
bash containers/mosaico/run build-image
bash containers/mosaico/run --profile claude-acp doctor
```

For delivery failures, correlate the accepted kind:9 id, target tag, RPC
session, and daemon delivery/completion log. A handshake proves the driver, not
mention delivery.

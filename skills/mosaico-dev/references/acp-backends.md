# ACP and app-server backends

Use this reference for Claude ACP, Codex app-server, Goose ACP, Hermes ACP, and
OpenCode ACP labs.
These transports use structured RPC instead of terminal-byte injection.

## Configuration contract

A structured launch requires two files:

```text
<MOSAICO_HOME>/harnesses.json
<MOSAICO_HOME>/agents/<slug>.json
```

`harnesses.json` contains only transport policy. For example:

```json
{
  "claude-acp": {
    "harness": "claude-code",
    "transport": "acp",
    "args": []
  }
}
```

`args` is optional and defaults to an empty array. It is the only bundle-owned
provider-argument surface. Agent profile names and executable selection do not
belong in a bundle; unknown fields are invalid.

The agent file selects that bundle:

```json
{
  "slug": "claude",
  "created_at": 0,
  "perSessionKey": true,
  "harness": "claude-acp"
}
```

This per-session agent is intentionally keyless on disk. Add `secret_key` and
`public_key` only with `perSessionKey: false` for a deliberately durable agent.

Safe inspection:

```bash
jq 'to_entries[] | {bundle:.key,harness:.value.harness,transport:.value.transport,args:(.value.args // [])}' \
  .container-state/claude-acp/mosaico/harnesses.json
jq '{slug,harness,profile,perSessionKey,has_secret:has("secret_key"),has_public:has("public_key")}' \
  .container-state/claude-acp/mosaico/agents/claude.json
```

For a normal per-session lab agent, both `has_secret` and `has_public` must be
false.

## Generated profiles

| profile | harness | transport | args override |
| --- | --- | --- | --- |
| `claude-acp` | `claude-code` | `acp` | `MOSAICO_DEV_CLAUDE_ACP_ARGS_JSON` |
| `codex-app-server` | `codex` | `app-server` | `MOSAICO_DEV_CODEX_APP_SERVER_ARGS_JSON` |
| `goose-acp` | `goose` | `acp` | `MOSAICO_DEV_GOOSE_ACP_ARGS_JSON` |
| `opencode-acp` | `opencode` | `acp` | `MOSAICO_DEV_OPENCODE_ACP_ARGS_JSON` |
| `hermes-acp` | `hermes` | `acp` | `MOSAICO_DEV_HERMES_ACP_ARGS_JSON` |

Default args are `[]`. Use the listed writer override only when the lab needs
explicit provider arguments. The value must be a JSON array of strings. There
are no model/profile objects.

A named Codex configuration is different from bundle args. Select it with:

```bash
MOSAICO_DEV_CODEX_CONFIG_PROFILE=planner \
  skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" codex-app-server
```

The writer places `"profile":"planner"` in the Codex agent file. Mosaico then
composes `$CODEX_HOME/planner.config.toml` over the base config in an isolated
app-server home. Codex app-server does not accept the native `--profile` flag.
Claude ACP and OpenCode ACP do not support named agent profiles. Goose 1.43.0
supports recipes through other CLI surfaces, but `goose acp` exposes no stable
recipe/profile selector. Omit `profile` for those combinations. Hermes ACP
accepts `MOSAICO_DEV_HERMES_PROFILE`; its `--profile <name>` selector is placed
before the `acp` subcommand.

## Smoke before launch

Generate the profile, run doctor, and drive the configured bundle:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" claude-acp
bash containers/mosaico/run --profile claude-acp doctor
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

A passing smoke proves initialization, session/thread creation, a real turn,
and a second real turn after cross-process resume. ACP uses `session/load`;
Codex app-server uses `thread/resume`.

Goose's canonical command is `goose acp`. Do not configure a PTY bundle or
native profile for this transport; it cannot activate either. Goose's own
plugins/extensions remain provider configuration, and `--with-builtin` can be
set through bundle `args` when a lab explicitly needs it. Mosaico's Goose Open
Plugin is still mandatory: its awaited hooks write full fabric context to the
session-specific Top Of Mind file before Goose calls the provider.

Hermes named profiles are discovered from `<HERMES_HOME>/profiles/<name>` and
activate natively as `hermes --profile <name> acp` for managed launch.
Prove the exact profile across both ACP processes with:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke hermes-acp \
  --profile builder
```

The smoke prints the resolved argv, first session ID, successful cross-process
`session/load`, both `end_turn` results, and `PASS`.

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

# ACP And App-Server Backends

Use this reference for structured Claude, Codex, and OpenCode live-lab runs.
These agents use JSON-RPC instead of terminal byte injection.

## Configuration Contract

The filename is `harnesses.json` (plural). A structured launch requires both:

```text
<TENEX_EDGE_HOME>/harnesses.json
<TENEX_EDGE_HOME>/agents/<slug>.json
```

The first file defines a named bundle:

```json
{
  "claude-acp": {
    "harness": "claude-code",
    "transport": "acp",
    "profile": { "model": "haiku" }
  }
}
```

The agent file selects it through `harness`:

```json
{
  "slug": "claude",
  "secret_key": "<secret>",
  "public_key": "<pubkey>",
  "created_at": 0,
  "perSessionKey": true,
  "harness": "claude-acp"
}
```

Never print the secret fields. Safe inspection:

```bash
jq 'to_entries[] | {bundle:.key,harness:.value.harness,transport:.value.transport,codex_config_profile:.value.codex_config_profile,profile:.value.profile}' \
  .container-state/claude-acp/tenex/edge/harnesses.json
jq '{slug,harness,perSessionKey}' \
  .container-state/claude-acp/tenex/edge/agents/claude.json
```

## Supported Structured Profiles

| profile | harness | transport | model override |
| --- | --- | --- | --- |
| `claude-acp` | `claude-code` | `acp` | `TENEX_EDGE_DEV_CLAUDE_ACP_MODEL` |
| `codex-app-server` | `codex` | `app-server` | `TENEX_EDGE_DEV_CODEX_APP_SERVER_MODEL` |
| `opencode-acp` | `opencode` | `acp` | `TENEX_EDGE_DEV_OPENCODE_ACP_MODEL` |

The defaults are the cheapest useful models known to the lab. If one is
rejected, set the corresponding variable and regenerate the profile. Provider
CLI flags do not belong after a structured `tenex-edge launch`; the bundle
profile is the source of truth.

Codex app-server bundles can also select an ordinary named Codex profile:

```bash
TENEX_EDGE_DEV_CODEX_CONFIG_PROFILE=planner \
  skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" codex-app-server
```

This writes `"codex_config_profile":"planner"` next to the bundle `profile`.
Codex app-server rejects the CLI `--profile` selector, so tenex-edge reads base
`config.toml`, deep-merges `planner.config.toml`, and materializes the result as
an isolated app-server `CODEX_HOME/config.toml`. Project configuration and the
bundle's inline `profile` (`-c` overrides) retain their normal higher
precedence. Profile names accept only letters, numbers, hyphens, and
underscores; missing or invalid profiles fail before launch.

When a named profile is selected, the writer lets its model stand unless
`TENEX_EDGE_DEV_CODEX_APP_SERVER_MODEL` is explicitly set. Approval and sandbox
lab overrides remain explicit. Host auth staging projects all
`~/.codex/*.config.toml` files into the isolated container home.

## Smoke Before Launch

Generate state, run doctor, then drive the exact configured bundle:

```bash
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude-acp
bash containers/tenex-edge/run --profile claude-acp doctor
skills/tenex-edge-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

A passing smoke proves `initialize`, session/thread creation, a real model turn,
and cross-process resume. ACP harnesses use `session/load`; Codex app-server
uses a fresh process, `thread/resume`, and a second real turn. Its smoke also
prints safe effective model, reasoning-effort, sandbox, and approval fields
from `config/read` so named-profile application is directly observable.

## Headless Launch

Register the mounted workspace before the first launch, then provide an initial
prompt through the tenex-edge launch surface:

```bash
bash containers/tenex-edge/run --profile claude-acp tenex-edge channel init
TENEX_EDGE_DEV_PROMPT="Run tenex-edge my session." \
  skills/tenex-edge-dev/scripts/launch-agent "${LAB_ENV}" launch claude-acp
```

Expected output includes `[tenex-edge acp] session: ...`. There is no PTY to
attach. The helper uses `tenex-edge-hosted` so the container remains alive after
the launch CLI returns; otherwise container lifecycle would reap the daemon and
RPC child. Do not start a second container against that profile while it is
live. Inspect bind-mounted logs and host-side `nak` probes, then stop it with
`cleanup-lab`.

## Troubleshooting

If the bundle does not resolve, compare the agent's `harness` value to the exact
key in `harnesses.json` and validate both files with `jq`. Rerun the writer when
they differ. Do not add `harness.json`, duplicate fields, or fallback commands.

If Claude asks `npx` for permission to install the adapter, rebuild the image:

```bash
bash containers/tenex-edge/run build-image
bash containers/tenex-edge/run --profile claude-acp doctor
```

Current doctor output includes `claude-acp-adapter`, Codex app-server, OpenCode
ACP, and parsed bundle/agent config. The adapter belongs in the image, not in an
interactive one-profile install.

For delivery failures, correlate the accepted kind:9 event id, inbox state,
headless session liveness, endpoint alias, and daemon delivery log. A passing
handshake alone proves the driver, not mention delivery.

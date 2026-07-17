# Container backends

This reference explains provider auth, isolated state, fabric identity, bundles,
and launch modes in the local lab.

## Auth boundary

The runner defaults to real host auth:

```bash
MOSAICO_CONTAINER_HOST_AUTH=1
```

Host credentials remain read-only. Writable hook, plugin, cache, and provider
state is staged under `.container-state/<profile>`. Do not run provider login
inside a disposable container, copy credentials into the repository, or print
auth files. Report a missing host path or failed provider check.

Codex staging includes base auth/config and top-level named `*.config.toml`
files. Claude may source current OAuth state from the macOS Keychain. OpenCode
uses its staged XDG config/data homes. Grok copies `auth.json` and optional
`config.toml` into writable isolated `GROK_HOME` without mutating host files.

## State boundary

Each profile owns:

```text
.container-state/<profile>/home/
.container-state/<profile>/cargo/
.container-state/<profile>/target/
.container-state/<profile>/mosaico/config.json
.container-state/<profile>/mosaico/harnesses.json
.container-state/<profile>/mosaico/agents/<slug>.json
.container-state/<profile>/mosaico/state.db*
.container-state/<profile>/mosaico/nmp.redb
```

`write-container-profiles` resets the disposable `mosaico/` directory by
default while preserving provider home and build caches. This clears both the
SQLite daemon state and NMP's `nmp.redb`; carrying either store onto a fresh
relay can produce stale membership or event projections. Set
`MOSAICO_DEV_RESET_PROFILE_STATE=0` only to diagnose that exact existing state.

Never run two containers against the same profile concurrently. Their daemons
share a bind-mounted socket/state path; the second can replace the socket,
evict the active agent, and make its hooks time out. While a launched agent is
alive, use host relay probes and bind-mounted logs only.

## Fabric identity

The writer creates a device config shaped like:

```json
{
  "whitelistedPubkeys": ["<human-owner-pubkey>"],
  "relays": ["ws://192.168.64.1:<lab-port>"],
  "indexerRelay": "ws://192.168.64.1:<lab-port>",
  "backendName": "claude-acp",
  "userNsec": "<relay-owner-secret>",
  "mosaicoPrivateKey": "<distinct-profile-backend-secret>"
}
```

`userNsec` represents the human operator. `mosaicoPrivateKey` represents the
backend and signs management operations and derives per-session identities.
They must not be the same key. The relay owner public key is the sole human
whitelist entry. Backend public keys are not human whitelist entries; their
authority comes from the backend management-key lifecycle.

Inspect only safe fields:

```bash
jq '{relays,indexerRelay,backendName,whitelistedPubkeys}' \
  .container-state/claude-acp/mosaico/config.json
```

## Bundle and agent ownership

Each bundle contains exactly:

```json
{
  "claude": {
    "harness": "claude-code",
    "transport": "pty",
    "args": []
  }
}
```

The agent file selects the bundle and optional native profile:

```json
{
  "slug": "claude",
  "created_at": 0,
  "perSessionKey": true,
  "harness": "claude"
}
```

Per-session agents omit key fields. Only durable `perSessionKey: false` agents
persist `secret_key` and `public_key`.

The bundle owns operational `args`; the agent owns the optional profile name.
The executable, required transport prefix, resume behavior, and profile
translation are code-owned. Unknown bundle fields fail parsing.

## Profile generation

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" \
  claude claude-acp codex codex-app-server grok opencode opencode-acp \
  codex-ollama opencode-ollama
```

Default bundle args are empty for standard PTY and structured profiles.
`codex-ollama` owns `--oss --local-provider ollama`; `opencode-ollama` owns
`-m <model>`, where `MOSAICO_DEV_OPENCODE_OLLAMA_MODEL` defaults to
`ollama/deepseek-r1:8b`.

Every profile has an exact JSON-array override:

| profile | args override |
| --- | --- |
| `claude` | `MOSAICO_DEV_CLAUDE_ARGS_JSON` |
| `claude-acp` | `MOSAICO_DEV_CLAUDE_ACP_ARGS_JSON` |
| `codex` | `MOSAICO_DEV_CODEX_ARGS_JSON` |
| `codex-app-server` | `MOSAICO_DEV_CODEX_APP_SERVER_ARGS_JSON` |
| `grok` | `MOSAICO_DEV_GROK_ARGS_JSON` |
| `opencode` | `MOSAICO_DEV_OPENCODE_ARGS_JSON` |
| `opencode-acp` | `MOSAICO_DEV_OPENCODE_ACP_ARGS_JSON` |
| `codex-ollama` | `MOSAICO_DEV_CODEX_OLLAMA_ARGS_JSON` |
| `opencode-ollama` | `MOSAICO_DEV_OPENCODE_OLLAMA_ARGS_JSON` |

Each override must be a JSON array of strings and replaces the generated args.

`MOSAICO_DEV_CODEX_CONFIG_PROFILE=<name>` adds the optional `profile` string to
the Codex app-server agent file. It does not add fields to the bundle.

## Direct and launched runs

Direct mode runs the provider CLI in the foreground and may receive provider
arguments:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct claude \
  -p "Respond with exactly OK." --model haiku
```

Use direct mode for auth, provider startup, and hook/plugin staging checks.

Launch mode takes no provider arguments:

```bash
bash containers/mosaico/run --profile claude mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude
```

Use launch mode for bundle/transport routing, native-profile activation,
hosted lifecycle, PTY behavior, and channel delivery. Configure provider flags
in bundle `args`, then regenerate the profile.

## Native profile inventory

`mosaico agents` without a target prints or interactively selects the full
launch inventory. It includes configured agents, eligible raw harnesses, and
native agent profiles discovered from:

- `$CODEX_HOME/agents` or `~/.codex/agents`
- `~/.claude/agents`
- `$XDG_CONFIG_HOME/opencode/agents` or `~/.config/opencode/agents`
- workspace-local `.codex/agents`, `.claude/agents`, and `.opencode/agents`

Workspace-local definitions override the matching global profile. A slug
provided by multiple harnesses appears as harness-suffixed choices; selecting a
choice persists the binding. Do not manufacture an agent JSON merely to hide an
inventory-routing failure.

## Prewarm and doctor

Run doctor against the exact generated profile:

```bash
bash containers/mosaico/run --profile claude-acp doctor
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

Doctor performs the Cargo build, installs the relevant Mosaico integration, and
checks provider auth plus transport tools. Structured smoke additionally proves
the configured RPC bundle. Do not use unsupported top-level version flags as a
prewarm shortcut.

For Grok, doctor must also verify `${GROK_HOME}/hooks/mosaico.json`; follow it
with the direct one-turn auth check in `grok-pty-lab.md`.

## Reporting

For each backend report the profile, bundle, transport, exact direct or launch
command, accepted model/provider args, auth result, PTY or RPC session id, logs
inspected, and the next concrete failure if it did not pass.

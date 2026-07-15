# Live Lab Workflow

This reference is the full procedure for proving mosaico behavior in a real,
inspectable local environment. Use it when the task is more than a quick CLI
check.

## What The Lab Proves

The live lab is meant to prove that the actual development stack works:

- croissant accepts the relay traffic mosaico produces
- containerized backends can reach the host relay
- real host AI credentials are visible inside the container state
- hooks/plugins are installed where the agent CLI will actually read them
- `mosaico launch` selects PTY or ACP/app-server from generated bundle config
- ACP/app-server handshakes, prompts, and resume work through `__acp-smoke`
- direct backend runs show the expected terminal UI and accept host auth
- injected context, fabric snapshots, and event flow are observable
- failures leave logs that a developer can inspect

Do not use this workflow to benchmark models or to prove high-quality agent
reasoning. Pick the cheapest model that can perform shell commands and summarize
what it sees.

## Initial Checks

Start from the repo root:

```bash
cd /path/to/mosaico
git status -sb
find skills/mosaico-dev -maxdepth 3 -type f -print | sort
```

Build and verify the container image:

```bash
bash containers/mosaico/run build-image
bash containers/mosaico/run doctor
```

`doctor` should verify container commands, `nak`, and the selected profile's
provider auth plus hook/plugin setup. If the doctor fails, use
`references/troubleshooting.md` before attempting a live run.

## Start The Relay

Run:

```bash
skills/mosaico-dev/scripts/start-croissant-relay
```

Expected output:

```text
run_id=...
env=/tmp/.../mosaico-live-lab-.../lab.env
relay=ws://192.168.64.1:<auto-port>
relay_pid=...
owner_pubkey=...
```

Keep the printed env path:

```bash
LAB_ENV=/tmp/.../lab.env
```

The relay command:

- uses `/tmp/croissant-smallmap` when present, else `${HOME}/Work/croissant`;
  set `MOSAICO_DEV_CROISSANT_DIR` to override
- derives the advertised relay host from the current Apple Container gateway,
  with `MOSAICO_DEV_RELAY_HOST` as an override
- binds to that host, with `MOSAICO_DEV_RELAY_BIND_HOST` as an override
- uses `MOSAICO_DEV_RELAY_PORT` or an unused high port from
  `MOSAICO_DEV_RELAY_PORT_BASE` (default `19888`)
- creates a temp work directory under `${TMPDIR:-/tmp}`
- creates a relay owner key without printing the secret
- starts croissant as a host process
- waits for NIP-11 before returning
- writes all run metadata, including `RELAY_LOG`, to `lab.env`

The auto port is intentional. Reusing a shared bridge port such as `9888` lets
stale live agents from older labs connect to the new relay and create the
`workspace` group before the current profile does. Pin a port only when that
specific shared-port behavior is under test:

```bash
MOSAICO_DEV_RELAY_PORT=9888 skills/mosaico-dev/scripts/start-croissant-relay
```

## Configure Backend Profiles

Single Claude ACP profile:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" claude-acp
```

Multiple profiles:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" \
  claude-acp codex-app-server opencode-acp
```

This writes:

```text
.container-state/<profile>/mosaico/config.json
.container-state/<profile>/mosaico/harnesses.json
.container-state/<profile>/mosaico/agents/<slug>.json
.container-state/<profile>/mosaico/
.container-state/<profile>/home/
```

The writer resets `.container-state/<profile>/mosaico` for every generated
lab profile, preserving provider home state and build cache while removing old
daemon DB/socket/log state. That reset is intentional: a fresh relay plus stale
workspace membership is not a valid live lab.

Every profile points only at the live croissant relay, with every generated
backend and agent pubkey whitelisted. `harnesses.json` defines the harness,
transport, inline profile, and optional Codex named profile; the agent file
selects it with `harness`. Fabric keys are disposable; provider credentials
still come from the host auth mounts.

After writing profiles, inspect the public shape without exposing secrets:

```bash
jq '{relays,indexerRelay,backendName,whitelistedPubkeys}' \
  .container-state/claude-acp/mosaico/config.json
jq 'to_entries[] | {bundle:.key,harness:.value.harness,transport:.value.transport,codex_config_profile:.value.codex_config_profile,profile:.value.profile}' \
  .container-state/claude-acp/mosaico/harnesses.json
jq '{slug,harness,perSessionKey}' \
  .container-state/claude-acp/mosaico/agents/claude.json
```

Do not print `userNsec` or `mosaicoPrivateKey`.

Prewarm the exact profile before opening an agent UI. This avoids confusing
cold Cargo builds with agent startup failures, and it proves staged auth before
the interactive run:

```bash
bash containers/mosaico/run --profile claude-acp doctor
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

The smoke proves the configured bundle: initialize, session/thread creation, a
real model prompt, and cross-process resume. Codex also reports safe effective
config fields before restarting, resuming its thread, and running a second turn.

## Direct Agent Runs

Use direct mode when testing the raw backend CLI plus container auth/hook
installation. Direct mode runs in the foreground terminal and is not
reattachable:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct claude --model haiku
```

Codex:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct codex -m gpt-5.3-codex-spark
```

OpenCode through the Ollama Cloud helper:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct opencode-ollama "${MOSAICO_OPENCODE_OLLAMA_MODEL:-ollama/deepseek-r1:8b}"
```

The helper records a container cidfile under the lab work directory when the
container runtime provides one. Use `scripts/cleanup-lab` so container-side
daemons do not continue retrying after the relay stops.

Do not run a second diagnostic container against the same profile while this
foreground agent is still active. If you need same-profile `mosaico` RPC
checks, stop the agent first or use the profile logs.

## PTY Launch Runs

Use launch mode when testing `mosaico launch` behavior, launch-time hook
setup, portable PTY integration, reattach, and context injection:

```bash
bash containers/mosaico/run --profile claude mosaico channel init
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude --model haiku
```

The command runs:

```text
bash containers/mosaico/run --profile claude mosaico launch claude -- --model haiku
```

The command starts a portable PTY supervisor, publishes the agent's public
kind:0 handle, and prints a line like:

```text
Launched amber-claude
```

Inspect or attach from another terminal with the same profile/home context:

```bash
bash containers/mosaico/run --profile claude mosaico sessions
```

## ACP/App-Server Launch Runs

Structured transports are headless. Configure the model in `harnesses.json`
through the profile writer, not with provider CLI flags. Before launching,
register the mounted workspace in that isolated profile:

```bash
bash containers/mosaico/run --profile claude-acp mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session and summarize the self header." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude-acp
```

The helper keeps the container alive after the headless CLI returns, because
the container owns the daemon and JSON-RPC child. Do not run a second container
against the same profile while it is live. Use host-side relay probes and the
bind-mounted profile logs, then stop it with `cleanup-lab`.

The helper passes `--headless` for structured profiles. A human running bare
`mosaico launch <agent>` on a TTY instead gets an Inquire picker over PTY
bundles, with headless launch as the final option.

## Multi-Agent Runs

For a multi-backend test:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" \
  claude-acp codex-app-server opencode-acp
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke codex-app-server
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke opencode-acp
```

Run structured smokes sequentially. For a live delivery test, initialize each
profile's workspace channel, launch each agent in a separate container, and use
relay events for cross-backend traffic. Keep prompts narrow; the goal is event,
transport, and hook proof, not task completion.

## Probe And Report

Capture the relay and event kinds:

```bash
skills/mosaico-dev/scripts/probe-lab "${LAB_ENV}"
```

Open the probe directory and inspect:

```text
nip11.json
relay.log
kind-39000.jsonl
kind-39001.jsonl
kind-39002.jsonl
kind-30315.jsonl
kind-9.jsonl
```

Your final report should name the feature under test and include the concrete
evidence surfaces. Do not summarize only from memory; cite the PTY id,
probe directory, and log files you generated.

## Cleanup

Stop sessions explicitly:

```bash
skills/mosaico-dev/scripts/cleanup-lab "${LAB_ENV}"
```

Remove disposable state only when it is no longer needed for debugging:

```bash
rm -rf .container-state/{claude,claude-acp,codex,codex-app-server,opencode,opencode-acp}
rm -rf "$(grep '^WORK_DIR=' "${LAB_ENV}" | cut -d= -f2- | xargs printf '%s')"
```

If a failure needs follow-up, preserve the work directory and report its path.

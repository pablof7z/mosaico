# Live Lab Workflow

This reference is the full procedure for proving tenex-edge behavior in a real,
inspectable local environment. Use it when the task is more than a quick CLI
check.

## What The Lab Proves

The live lab is meant to prove that the actual development stack works:

- croissant accepts the relay traffic tenex-edge produces
- containerized backends can reach the host relay
- real host AI credentials are visible inside the container state
- hooks/plugins are installed where the agent CLI will actually read them
- `tenex-edge launch` and direct backend runs show the expected terminal UI
- injected context, fabric snapshots, and event flow are observable
- failures leave logs that a developer can inspect

Do not use this workflow to benchmark models or to prove high-quality agent
reasoning. Pick the cheapest model that can perform shell commands and summarize
what it sees.

## Initial Checks

Start from the repo root:

```bash
cd /path/to/tenex-edge
git status -sb
find skills/tenex-edge-dev -maxdepth 3 -type f -print | sort
```

Build and verify the container image:

```bash
bash containers/tenex-edge/run build-image
bash containers/tenex-edge/run doctor
```

`doctor` should verify container commands, provider auth projections, `nak`,
and hook/plugin setup. If the doctor fails, use
`references/troubleshooting.md` before attempting a live run.

## Start The Relay

Run:

```bash
skills/tenex-edge-dev/scripts/start-croissant-relay
```

Expected output:

```text
run_id=...
env=/tmp/.../tenex-edge-live-lab-.../lab.env
relay=ws://192.168.64.1:<auto-port>
relay_pid=...
owner_pubkey=...
```

Keep the printed env path:

```bash
LAB_ENV=/tmp/.../lab.env
```

The relay command:

- uses `${HOME}/Work/croissant` unless `TENEX_EDGE_DEV_CROISSANT_DIR` is set
- binds to `TENEX_EDGE_DEV_RELAY_HOST` or the Apple container bridge IP
- uses `TENEX_EDGE_DEV_RELAY_PORT` or an unused high port from
  `TENEX_EDGE_DEV_RELAY_PORT_BASE` (default `19888`)
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
TENEX_EDGE_DEV_RELAY_PORT=9888 skills/tenex-edge-dev/scripts/start-croissant-relay
```

## Configure Backend Profiles

Single Claude profile:

```bash
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude
```

Multiple profiles:

```bash
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude codex opencode
```

This writes:

```text
.container-state/<profile>/tenex/config.json
.container-state/<profile>/tenex/edge/
.container-state/<profile>/home/
```

The writer resets `.container-state/<profile>/tenex/edge` for every generated
lab profile, preserving provider home state and build cache while removing old
daemon DB/socket/log state. That reset is intentional: a fresh relay plus stale
workspace membership is not a valid live lab.

Every profile points only at the live croissant relay. Every generated backend
pubkey is whitelisted in every generated profile, so the backends can see each
other in the local fabric. The generated Nostr keys are disposable fabric keys;
the AI provider credentials still come from the host auth mounts.

After writing profiles, inspect the public shape without exposing secrets:

```bash
jq '{relays,indexerRelay,backendName,whitelistedPubkeys}' .container-state/claude/tenex/config.json
```

Do not print `userNsec` or `tenexPrivateKey`.

Prewarm the exact profile before opening an agent UI. This avoids confusing
cold Cargo builds with agent startup failures, and it proves staged auth before
the interactive run:

```bash
bash containers/tenex-edge/run --profile claude doctor
bash containers/tenex-edge/run --profile claude claude -p "Respond with exactly OK." --model haiku
```

Use the same pattern for other providers with their cheapest working command.

## Direct Agent Runs

Use direct mode when testing the raw backend CLI plus container auth/hook
installation. Direct mode runs in the foreground terminal and is not
reattachable:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" direct claude --model haiku
```

Codex:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" direct codex -m gpt-5.3-codex-spark
```

OpenCode through the Ollama Cloud helper:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" direct opencode-ollama "${TENEX_EDGE_OPENCODE_OLLAMA_MODEL:-ollama/deepseek-r1:8b}"
```

The helper records a container cidfile under the lab work directory when the
container runtime provides one. Use `scripts/cleanup-lab` so container-side
daemons do not continue retrying after the relay stops.

Do not run a second diagnostic container against the same profile while this
foreground agent is still active. If you need same-profile `tenex-edge` RPC
checks, stop the agent first or use the profile logs.

## Launch Mode Runs

Use launch mode when testing `tenex-edge launch` behavior, launch-time hook
setup, portable PTY integration, reattach, and context injection:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" launch claude --model haiku
```

The command runs:

```text
bash containers/tenex-edge/run --profile claude tenex-edge launch claude -- --model haiku
```

The command starts a portable PTY supervisor, prints a line like:

```text
[tenex-edge pty] session: claude-...
```

Save that id:

```bash
PTY_ID=claude-...
```

Inspect or drive it from another terminal with the same profile/home context:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge pty list
bash containers/tenex-edge/run --profile claude tenex-edge pty attach "${PTY_ID}"
bash containers/tenex-edge/run --profile claude tenex-edge pty inject "${PTY_ID}" \
  "Run tenex-edge who and summarize the self header."
```

## Multi-Agent Runs

For a multi-backend test:

```bash
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude codex opencode
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" launch claude --model haiku
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" launch codex -m gpt-5.3-codex-spark
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" launch opencode-ollama "${TENEX_EDGE_OPENCODE_OLLAMA_MODEL:-ollama/deepseek-r1:8b}"
```

Use separate portable PTY sessions for every backend. Send small prompts that force
observable fabric behavior, such as running `tenex-edge who`, posting a project
message, or responding to a mention. Keep prompts narrow; the goal is event and
hook proof, not task completion.

## Probe And Report

Capture the relay and event kinds:

```bash
skills/tenex-edge-dev/scripts/probe-lab "${LAB_ENV}"
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
skills/tenex-edge-dev/scripts/cleanup-lab "${LAB_ENV}"
```

Remove disposable state only when it is no longer needed for debugging:

```bash
rm -rf .container-state/claude .container-state/codex .container-state/opencode
rm -rf "$(grep '^WORK_DIR=' "${LAB_ENV}" | cut -d= -f2- | xargs printf '%s')"
```

If a failure needs follow-up, preserve the work directory and report its path.

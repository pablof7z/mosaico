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
cd /Users/pablofernandez/src/tenex-edge
git status -sb
find skills/tenex-edge-dev -maxdepth 3 -type f -print | sort
```

Build and verify the container image:

```bash
bash containers/tenex-edge/run build-image
bash containers/tenex-edge/run doctor
```

`doctor` should verify container commands, provider auth projections, `nak`,
`tmux`, and hook/plugin setup. If the doctor fails, use
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
relay=ws://192.168.64.1:9888
relay_tmux=te-relay-...
owner_pubkey=...
```

Keep the printed env path:

```bash
LAB_ENV=/tmp/.../lab.env
```

The relay command:

- uses `/Users/pablofernandez/Work/croissant`
- binds to `TENEX_EDGE_DEV_RELAY_HOST` or the Apple container bridge IP
- uses `TENEX_EDGE_DEV_RELAY_PORT` or `9888`
- creates a temp work directory under `${TMPDIR:-/tmp}`
- creates a relay owner key without printing the secret
- starts croissant in a host tmux session
- waits for NIP-11 before returning
- writes all run metadata to `lab.env`

If port `9888` is in use, set a different port:

```bash
TENEX_EDGE_DEV_RELAY_PORT=9899 skills/tenex-edge-dev/scripts/start-croissant-relay
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

Every profile points only at the live croissant relay. Every generated backend
pubkey is whitelisted in every generated profile, so the backends can see each
other in the local fabric. The generated Nostr keys are disposable fabric keys;
the AI provider credentials still come from the host auth mounts.

After writing profiles, inspect the public shape without exposing secrets:

```bash
jq '{relays,indexerRelay,backendName,whitelistedPubkeys}' .container-state/claude/tenex/config.json
```

Do not print `userNsec` or `tenexPrivateKey`.

## Direct Agent Runs

Use direct mode when testing the raw backend CLI plus container auth/hook
installation:

```bash
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" direct claude --model haiku
```

Codex:

```bash
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" direct codex -m gpt-5.3-codex-spark
```

OpenCode through the Ollama Cloud helper:

```bash
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" direct opencode-ollama "${TENEX_EDGE_OPENCODE_OLLAMA_MODEL:-ollama/deepseek-r1:8b}"
```

The helper prints the tmux session name. Save it:

```bash
AGENT_TMUX=te-direct-claude-...
```

Drive the session:

```bash
tmux send-keys -t "${AGENT_TMUX}" "Run tenex-edge who and summarize the self header." C-m
```

Read the session:

```bash
tmux capture-pane -pt "${AGENT_TMUX}" -S -240 -e
```

## Launch Mode Runs

Use launch mode when testing `tenex-edge launch` behavior, launch-time hook
setup, tmux integration, and context injection:

```bash
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" launch claude --model haiku
```

The command runs:

```text
bash containers/tenex-edge/run --profile claude tenex-edge launch claude -- --model haiku
```

inside a host tmux session. Inspect the host tmux pane first. If
`tenex-edge launch` creates or names a nested tmux session, inspect that nested
session too.

## Multi-Agent Runs

For a multi-backend test:

```bash
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude codex opencode
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" launch claude --model haiku
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" launch codex -m gpt-5.3-codex-spark
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" launch opencode-ollama "${TENEX_EDGE_OPENCODE_OLLAMA_MODEL:-ollama/deepseek-r1:8b}"
```

Use separate tmux sessions for every backend. Send small prompts that force
observable fabric behavior, such as running `tenex-edge who`, posting a project
message, or responding to a mention. Keep prompts narrow; the goal is event and
hook proof, not task completion.

## Probe And Report

Capture the relay, event kinds, and agent panes:

```bash
skills/tenex-edge-dev/scripts/probe-lab "${LAB_ENV}" "${AGENT_TMUX}"
```

For multiple sessions:

```bash
skills/tenex-edge-dev/scripts/probe-lab "${LAB_ENV}" "${CLAUDE_TMUX}" "${CODEX_TMUX}" "${OPENCODE_TMUX}"
```

Open the probe directory and inspect:

```text
nip11.json
relay-pane.txt
kind-39000.jsonl
kind-39001.jsonl
kind-39002.jsonl
kind-30315.jsonl
kind-9.jsonl
tmux-<session>.txt
```

Your final report should name the feature under test and include the concrete
evidence surfaces. Do not summarize only from memory; cite the tmux/probe/log
files you generated.

## Cleanup

Stop sessions explicitly:

```bash
tmux kill-session -t "${AGENT_TMUX}"
tmux kill-session -t "$(grep '^RELAY_TMUX=' "${LAB_ENV}" | cut -d= -f2- | xargs printf '%s')"
```

Remove disposable state only when it is no longer needed for debugging:

```bash
rm -rf .container-state/claude .container-state/codex .container-state/opencode
rm -rf "$(grep '^WORK_DIR=' "${LAB_ENV}" | cut -d= -f2- | xargs printf '%s')"
```

If a failure needs follow-up, preserve the work directory and report its path.

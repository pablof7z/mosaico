# Live lab workflow

This procedure proves Mosaico on a real local relay and real provider CLIs.

## 1. Check the repository and image

From the repository root:

```bash
git status -sb
find skills/mosaico-dev -maxdepth 3 -type f -print | sort
bash containers/mosaico/run build-image
bash containers/mosaico/run doctor
```

Doctor must verify the installed CLIs, structured transport commands, `nak`,
provider auth projection, and Mosaico hooks/plugins. Resolve doctor failures
before opening an agent UI.

## 2. Start an isolated relay

```bash
skills/mosaico-dev/scripts/start-croissant-relay
```

If the runner reaps background descendants, set
`MOSAICO_DEV_RELAY_FOREGROUND=1` and clean up from another terminal.

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
LAB_ENV=/tmp/mosaico-live-lab-.../lab.env
```

The helper chooses an unused high port by default, binds croissant to the Apple
container bridge, waits for NIP-11, and records the relay owner identity without
printing its secret. A fresh port prevents stale agents from older runs from
claiming the new workspace first. Pin `MOSAICO_DEV_RELAY_PORT` only when shared
port behavior is itself under test.

## 3. Generate current profile state

Single profile:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" claude-acp
```

Multi-provider lab:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" \
  claude claude-acp codex codex-app-server grok goose-acp hermes hermes-acp \
  opencode opencode-acp
```

Each profile receives:

```text
.container-state/<profile>/mosaico/config.json
.container-state/<profile>/mosaico/harnesses.json
.container-state/<profile>/mosaico/agents/<slug>.json
```

The writer resets profile-local Mosaico state by default, including `state.db*`,
the daemon socket/logs, sessions, and `nmp.redb`. It preserves provider home and
build caches.

The device config uses the relay owner as the human `userNsec` and a distinct
per-profile backend key as `mosaicoPrivateKey`. Generated per-session agent files
are keyless. Inspect the public shape:

```bash
jq '{relays,indexerRelay,backendName,whitelistedPubkeys}' \
  .container-state/claude-acp/mosaico/config.json
jq 'to_entries[] | {bundle:.key,harness:.value.harness,transport:.value.transport,args:(.value.args // [])}' \
  .container-state/claude-acp/mosaico/harnesses.json
jq '{slug,harness,profile,perSessionKey,has_secret:has("secret_key"),has_public:has("public_key")}' \
  .container-state/claude-acp/mosaico/agents/claude.json
```

Do not print `userNsec`, `mosaicoPrivateKey`, or provider auth files.

## 4. Prewarm the exact profile

For ACP/app-server:

```bash
bash containers/mosaico/run --profile claude-acp doctor
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

Use `hermes-acp` in the same commands to prove native Hermes ACP initialization,
turns, persisted `session/load` resume, and the installed Mosaico plugin.

The smoke proves the configured bundle, initialization, a real model turn, and
resume. For PTY, exact-profile doctor performs the build and integration
install; optionally use a tiny direct prompt to prove provider auth.

## 5. Choose a run mode

### Direct provider check

Direct mode is foreground and may receive provider CLI args:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct claude \
  -p "Respond with exactly OK." --model haiku
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct codex \
  -m gpt-5.3-codex-spark
```

Use it for auth and integration staging. It does not prove Mosaico hosted
routing or hosted lifecycle.

### PTY launch

Register the workspace, then launch without provider args:

```bash
bash containers/mosaico/run --profile claude mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude
```

The bundle's `transport: "pty"` selects portable PTY hosting. The current launch
surface is `mosaico <target> [prompt] [-- <args>...]`; durable provider flags
belong in bundle `args`, while separator arguments apply to one launch. Use the
attached terminal for UI evidence.

Use `grok-pty-lab.md` for native Grok hook provenance and p-tagged injection
proof.

### ACP/app-server launch

```bash
bash containers/mosaico/run --profile claude-acp mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session and summarize the self header." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude-acp
```

The bundle transport selects ACP or app-server. The helper keeps that container
alive after the launch command returns because it owns the daemon and RPC child.
Expected output contains an RPC session id; there is no PTY.

## 6. Validate launch inventory when relevant

Run a targetless launch in the generated profile:

```bash
bash containers/mosaico/run --profile codex mosaico agents
```

In a non-interactive command this prints available launch targets and exits. In
a terminal it opens the fuzzy selector. The inventory includes configured
agents, eligible raw harnesses, installed global/workspace native profiles, and
Hermes named profiles. Test both a single-harness profile and, when available,
a same-slug cross-harness profile. The latter must print/select
harness-suffixed targets and persist the chosen binding.

## 7. Deliver a tagged mention

The supported mention surface uses structured tags:

```bash
mosaico channel send --channel <channel> --tag <session-handle> \
  --message "Run mosaico my session."
```

Do this from a separate sender profile or external installed backend, not by
starting a second container against the live target profile. A literal
`@handle` in the message is rejected as ambiguous unless `--force` is used; it
does not create a recipient tag.

## 8. Inspect safely

While a launched container is alive, use only host-side surfaces:

```bash
skills/mosaico-dev/scripts/probe-lab "${LAB_ENV}"
tail -n 200 .container-state/claude-acp/mosaico/daemon.log
tail -n 200 .container-state/claude-acp/mosaico/relay.log
source "${LAB_ENV}"
tail -n 300 "${RELAY_LOG}"
```

Do not run another `containers/mosaico/run --profile <live-profile>` command,
including a bare `mosaico` invocation, `channel`, `debug explain`, or `debug
hook-tail`. A second daemon can replace the socket and destroy the live agent's
delivery path. Stop the launched container first if same-profile CLI inspection is required.

After stopping it, supported diagnostics include:

```bash
bash containers/mosaico/run --profile claude mosaico
bash containers/mosaico/run --profile claude mosaico debug explain event:<id>
bash containers/mosaico/run --profile claude mosaico debug hook-tail
```

## 9. Multi-agent runs

Smoke structured profiles sequentially. For live delivery, launch each target
in its own profile/container and use relay events or a separate sender profile
for cross-agent traffic. Keep prompts narrow; prove transport, event, and hook
behavior rather than task sophistication.

## 10. Report and clean up

Capture:

```bash
skills/mosaico-dev/scripts/probe-lab "${LAB_ENV}"
```

Report the relay/run id, profiles and bundle metadata, exact commands, direct or
launch mode, PTY/RPC session ids, auth result, relay/event evidence, log paths,
and feature-specific result.

Stop containers before the relay:

```bash
skills/mosaico-dev/scripts/cleanup-lab "${LAB_ENV}"
```

Keep failed-run state for diagnosis. When deleting a disposable profile
manually, remove that exact `.container-state/<profile>` only; do not use a
broad recursive target.

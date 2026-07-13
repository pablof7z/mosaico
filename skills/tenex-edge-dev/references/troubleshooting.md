# Troubleshooting

Use this when the live lab does not start cleanly or evidence is missing.

## Wrong Identity Command

Use:

```bash
tenex-edge who
```

Do not use obsolete identity subcommands. If old command names appear in active
skill/docs/scripts outside historical wiki material, remove them.

Check:

```bash
grep -R "obsolete command pattern" skills containers e2e docs --exclude-dir=target
```

Replace the pattern with the actual stale string only in your local shell when
auditing; do not add stale vocabulary back to committed files.

## Relay Port Already In Use

Symptom:

```text
port ... is already held by pid ...
```

Find it:

```bash
lsof -nP -iTCP:<port> -sTCP:LISTEN
```

By default `start-croissant-relay` auto-selects an unused high port so stale
labs do not share a relay. If a fixed port is required, set it explicitly:

```bash
TENEX_EDGE_DEV_RELAY_PORT=9899 skills/tenex-edge-dev/scripts/start-croissant-relay
```

Only kill an existing process if you know it belongs to a stale test.

## Relay Does Not Become Ready

Inspect the relay log:

```bash
tail -n 120 "${RELAY_LOG}"
```

Check:

- croissant checkout exists at `${HOME}/Work/croissant` or
  `TENEX_EDGE_DEV_CROISSANT_DIR`
- `go build` succeeds there
- `HOST` is the Apple container bridge IP, usually `192.168.64.1`
- `PORT` is not in use
- `DATAPATH` is writable
- `OWNER_PUBLIC_KEY` is a hex public key

Then retry with a fresh run id or port.

## Container Cannot Reach Relay

Host reachability:

```bash
curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" | jq .
```

Container reachability:

```bash
bash containers/tenex-edge/run --profile claude shell -c "curl -fsS -H 'Accept: application/nostr+json' '${RELAY_HTTP}'"
```

If host works and container fails:

- verify croissant was bound to the bridge IP, not only localhost
- verify the profile config uses `ws://<bridge-ip>:<port>`
- verify Apple container networking is running
- try a fresh port

## Host Auth Missing

Run:

```bash
bash containers/tenex-edge/run doctor
```

If it reports a missing host auth path:

- report the path
- do not run provider login inside the container
- do not create replacement provider files in the repo
- do not copy credential contents into `.container-state` by hand

The expected fix is host-side auth or host-auth projection repair.

## Claude Stops At OAuth

Symptom: the Claude terminal UI shows a login, OAuth, paste-code, or first-run
auth prompt instead of accepting the prompt.

Treat this as a failed auth staging check. Do not paste OAuth codes, do not run
Claude login in the container, and do not print credential files. Check:

```bash
bash containers/tenex-edge/run --profile claude doctor
bash containers/tenex-edge/run --profile claude claude -p "Respond with exactly OK." --model haiku
```

On macOS the runner should prefer the `Claude Code-credentials` Keychain item
over a stale host `~/.claude/.credentials.json`. If the tiny command fails,
fix `containers/tenex-edge/host-auth.bash` or host auth before running the lab.

## Host Hook Path Leaked Into Claude

Symptom:

```text
spawning detached daemon from /Users/.../.local/bin/tenex-edge: No such file or directory
```

The staged Claude settings are still carrying a host `TENEX_EDGE_BIN`. The
container profile must use:

```text
/state/target/debug/tenex-edge
```

Rerun the profile after staging has sanitized Claude settings. Do not edit host
Claude settings just to make a container lab pass.

## Launch Session Has No Session Anchor

Symptom: Claude launches and is authenticated, but the statusline says:

```text
[te: @te_session not set ...]
```

This means the launch session did not get a successful SessionStart hook before the
statusline rendered. A brief startup frame can show this before SessionStart
settles; it is a failure when the warning persists after the prompt is accepted
or after the agent runs `tenex-edge who`. Check that auth staging did not
overwrite installed hooks without reinstalling them:

```bash
jq '.hooks | keys' .container-state/<profile>/home/.claude/settings.json
find .container-state/<profile>/tenex/edge/sessions -name hook-calls.jsonl -print
```

For Claude launch mode, `containers/tenex-edge/run tenex-edge ...` must install
the `claude-code` harness after staging auth, even though the agent slug is
`claude`. A passing session shows an identity like
`claude1@<profile> workspace workspace [idle]` instead of the warning.

## Claude Hooks Cannot Install

Common cause: Claude settings were mounted read-only. The host-auth staging
should copy writable settings into profile state while keeping credentials
read-only.

Check:

```bash
bash containers/tenex-edge/run --profile claude doctor
find .container-state/claude -maxdepth 4 -type f | sort
```

If hooks fail, inspect the staged Claude settings path and file permissions. Do
not make host credential directories writable from the container.

## Model Flag Rejected

Capture the exact CLI output from the foreground terminal or attached PTY:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge pty attach "${PTY_ID}"
```

Then retry with the cheapest model the installed CLI accepts. Record both the
rejected flag and the fallback. The lab should continue unless the model choice
itself is what you are testing.

## Cargo Or Build Cache Problems

If Rust or Go build caches fail with transient corruption:

```bash
cargo test --no-run
```

or rebuild the specific binary/image that failed. Do not delete broad cache
directories unless the user asks or the failure clearly points to that cache.
For fresh profiles, a long first run is usually a cold build, not a hung agent.
Prewarm the exact profile before pty launch.

## Stale Daemon Socket Or State

Apple containers can leave a Unix socket path in the bind-mounted profile state
after a container exits. The runner removes the socket before new sequential
runs by default, but an active launched agent can still own the same profile.
Avoid same-profile parallel commands.

If an interrupted disposable profile has a corrupt or half-created daemon DB,
use a fresh profile or remove only that generated profile's daemon state:

```bash
rm -f .container-state/<profile>/tenex/edge/daemon.sock
rm -f .container-state/<profile>/tenex/edge/state.db*
```

Do not remove a non-disposable profile unless the user approves it.

## Management Key Is Not Admin

Symptom:

```text
ensure_channel_ready: management key is not admin of "workspace"
blocked: unknown member
not in channel workspace
```

This usually means a profile config was pointed at a fresh relay while the
profile-local daemon DB still reflected an older workspace and key set. For
live labs, rerun:

```bash
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude
```

The writer resets `.container-state/claude/tenex/edge` by default. If you set
`TENEX_EDGE_DEV_RESET_PROFILE_STATE=0`, use a fresh profile name instead.

If the profile state is already fresh, check for shared relay contamination:

```bash
nak req -k 39000 "${RELAY_WS}" | head
tail -n 160 "${RELAY_LOG}"
```

A fresh lab should not show old profile pubkeys creating `workspace` before the
current profile connects. Use the default auto port, or explicitly choose a new
port; do not keep retesting on a shared `9888` relay with stale agents alive.

If the lab otherwise passed and `blocked: unknown member` appears only in
`.container-state/<profile>/tenex/edge/logs/group-mgmt.log` as
`9000 put-admin (self-grant via userNsec)`, treat it as a non-blocking cleanup
or background repair warning. It becomes a failing symptom when it is paired
with `management key is not admin`, `not in channel workspace`, a missing
session anchor, or missing channel/member events.

After a launch-mode lab, run cleanup before stopping or reusing the relay:

```bash
skills/tenex-edge-dev/scripts/cleanup-lab "${LAB_ENV}"
```

## Croissant pprof Conflict

Croissant can log a local pprof bind conflict on `127.0.0.1:3337` while the
relay itself is healthy on the requested bridge URL. Confirm actual health with:

```bash
curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" | jq .
nak req -k 39000 "${RELAY_WS}" | head
```

Treat pprof as fatal only if NIP-11 or WebSocket probes also fail.

## Relay Has No Events

Check in order:

1. Agent actually launched and accepted the prompt.
2. Agent profile config points at the croissant relay.
3. The backend profile has a generated key and whitelist.
4. Croissant relay log shows a subscription or connection.
5. `nak req` is pointed at the same relay URL.
6. Hook/daemon logs show the action that should have published.

An empty `nak` output is useful only if paired with these checks.

## Agent UI Is Not Inspectable

PTY reattach and injection evidence requires a PTY profile in launch mode. If a
PTY agent was started as a direct foreground run, stop it and relaunch through:

```bash
skills/tenex-edge-dev/scripts/launch-agent "${LAB_ENV}" launch claude --model haiku
```

Use:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge pty list
bash containers/tenex-edge/run --profile claude tenex-edge pty attach <pty-id>
```

## Stale Active Strings

Before reporting the skill clean, run the repo audit the user expects:

```bash
grep -R "stale string" * | grep -v docs/wiki | wc -l
```

Replace `stale string` locally with the exact deprecated term being audited.
The count should be zero for active source/docs. Keep historical wiki material
out of this cleanup unless the user asks.

## Thin Report

If the report only says "passed" or "failed", it is not done. Add:

- relay URL and run id
- profile names
- PTY ids or ACP headless session ids
- exact launch commands
- probe directory
- croissant evidence
- agent UI evidence
- hook/log evidence when relevant
- next failing command if not passing

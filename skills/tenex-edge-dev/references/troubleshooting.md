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
port 9888 is already held by pid ...
```

Find it:

```bash
lsof -nP -iTCP:9888 -sTCP:LISTEN
```

Prefer a new port for the lab:

```bash
TENEX_EDGE_DEV_RELAY_PORT=9899 skills/tenex-edge-dev/scripts/start-croissant-relay
```

Only kill an existing process if you know it belongs to a stale test.

## Relay Does Not Become Ready

Capture the relay pane:

```bash
tmux capture-pane -pt "${RELAY_TMUX}" -S -120 -e
```

Check:

- croissant checkout exists at `/Users/pablofernandez/Work/croissant`
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
bash containers/tenex-edge/run --profile claude sh -lc 'curl -fsS -H "Accept: application/nostr+json" http://192.168.64.1:9888'
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

Capture the exact CLI output from tmux:

```bash
tmux capture-pane -pt "${AGENT_TMUX}" -S -120 -e
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

## Relay Has No Events

Check in order:

1. Agent actually launched and accepted the prompt.
2. Agent profile config points at the croissant relay.
3. The backend profile has a generated key and whitelist.
4. Croissant tmux pane shows a subscription or connection.
5. `nak req` is pointed at the same relay URL.
6. Hook/daemon logs show the action that should have published.

An empty `nak` output is useful only if paired with these checks.

## Agent UI Is Not Inspectable

Every run should be inside host tmux. If an agent was started in the foreground,
stop and relaunch through:

```bash
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" direct claude --model haiku
```

or:

```bash
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" launch claude --model haiku
```

Use:

```bash
tmux list-sessions
tmux capture-pane -pt <session> -S -240 -e
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
- tmux session names
- exact launch commands
- probe directory
- croissant evidence
- agent UI evidence
- hook/log evidence when relevant
- next failing command if not passing

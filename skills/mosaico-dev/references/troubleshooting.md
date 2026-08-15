# Troubleshooting

Use the first failing command and the real profile/relay state. Do not add
compatibility fields or launch overrides to work around current contract errors.

## Relay does not start

For a fixed-port conflict:

```bash
lsof -nP -iTCP:<port> -sTCP:LISTEN
```

Only stop a process known to be a stale lab. The relay helper normally chooses
an unused high port. If readiness times out, inspect `${RELAY_LOG}` and verify
the croissant checkout, bridge bind address, writable data path, owner public
key, and NIP-11 response.

## Container cannot reach the relay

```bash
curl -fsS -H 'Accept: application/nostr+json' "${RELAY_HTTP}" | jq .
bash containers/mosaico/run --profile claude shell -c \
  "curl -fsS -H 'Accept: application/nostr+json' '${RELAY_HTTP}'"
```

Run the container check before launching that profile. If host reachability
passes but container reachability fails, verify bridge binding, profile relay
URL, Apple container networking, and the chosen port.

## Host auth missing or Claude opens OAuth

```bash
bash containers/mosaico/run --profile claude doctor
bash containers/mosaico/run --profile claude claude -p \
  "Respond with exactly OK." --model haiku
```

Treat login, OAuth, paste-code, or first-run prompts as auth-staging failures.
Do not authenticate inside the disposable container or print credential files.
On macOS, Claude staging should prefer the current Keychain credential over a
stale JSON credential.

## Host hook path leaked into the container

If a hook tries to execute a host path, verify staged provider settings point to:

```text
/state/target/debug/mosaico
```

Repair host-auth staging and regenerate the profile; do not change host hook
settings merely to pass the lab.

## Preset or agent config is rejected

Validate the exact schema:

```bash
jq . .container-state/<profile>/mosaico/presets.json
jq '{slug,harness,preset,profile,perSessionKey,has_secret:has("secret_key"),has_public:has("public_key")}' \
  .container-state/<profile>/mosaico/agents/<slug>.json
```

Each preset maps canonical harness names to optional `pty`, `acp`, and
`app-server` string arrays. The agent owns canonical `harness`, optional
`preset`, and optional `profile`. A `perSessionKey: true` agent must be keyless.

## Launch arguments are rejected

The current surface is:

```text
mosaico <TARGET> [PROMPT] [--channel [ROOM]] [--name ...] [-- <ARGS>...]
```

Reusable provider flags belong in preset argument arrays. One-launch arguments
must follow `--`; options before it belong to Mosaico. Direct mode receives
provider CLI arguments without the Mosaico separator.

The launch workspace always resolves from the current directory. Change into
the intended workspace before launching; there is no launch `--workspace` flag.

## Launch target is missing or ambiguous

```bash
bash containers/mosaico/run --profile <profile> mosaico agents
```

A non-interactive run prints available targets. Check live harness detection,
configured agents, and the installed global/workspace native agent directories.
`presets.json` is optional argument policy, not catalog membership. If the same
native slug exists in several harnesses, use the harness-suffixed target.

## Workspace is unknown

Register the mounted workspace once per fresh profile:

```bash
bash containers/mosaico/run --profile claude mosaico channel init
```

The profile-local workspace registry is independent of Git discovery.

## Session has no anchor

If the UI persistently shows no session id after the first prompt or after
`mosaico my session`, inspect installed hooks and hook-call files:

```bash
jq '.hooks | keys' .container-state/<profile>/home/.claude/settings.json
find .container-state/<profile>/mosaico/sessions -name hook-calls.jsonl -print
```

Doctor/install must use the canonical harness name (`claude-code`, `codex`,
`grok`, `goose`, `hermes`, `kimi`, or `opencode`) even when the public agent slug differs. Goose
requires Mosaico's Open Plugin and native Top Of Mind extension in addition to
its ACP command and staged auth/config. A Goose launch that lacks that plugin,
has it disabled, disables Top Of Mind, or runs Goose older than 1.43.0 must fail
before creating a session.

## Same-profile inspection broke a live agent

Symptoms include `[mosaico: down]`, a hook timeout, socket replacement, or
startup cleanup removing the active agent. Stop the launched container with the
recorded cidfile/cleanup helper. Do not attempt to recover it through another
same-profile command. After it is stopped, remove only a stale socket if needed
and relaunch.

While an agent is alive, only host log reads, croissant logs, `nak`, and
`probe-lab` are safe. Same-profile `sessions`, `channel`, `debug explain`, and
`debug hook-tail` must wait until the launch container stops.

## Stale SQLite or NMP state

A fresh relay paired with old `state.db` or `nmp.redb` can retain obsolete
workspace, membership, or acquisition state. For a disposable lab, rerun:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" <profile>
```

The profile writer's default reset recreates the disposable profile. To repair
an already-stopped disposable profile directly, use Mosaico's owned reset door
and target only that profile:

```bash
MOSAICO_HOME=.container-state/<profile>/mosaico mosaico daemon reset-state --yes-i-know-this-wipes-local-state
```

Never delete these from a live or non-disposable profile without explicit
authorization.

## Daemon will not start and NMP refuses the store

Read the selected instance's `daemon.log`. A refused store is a daemon that
exits before it can answer an RPC; its structured startup error carries the
exact `condition`, `summary`, and `fix`:

- `superseded-epoch` — the store is not this build's schema epoch. NMP migrates
  nothing across an epoch and reads **nothing** inside a store it refused, so no
  tool can tell you what the file holds; "no readable marker" means *not this
  epoch*, not *empty*. Run `mosaico daemon reset-state
  --yes-i-know-this-wipes-local-state`, then restart. This clears all runtime
  state for the selected instance while preserving configuration. The
  relay-backed read cache re-acquires; any write NMP had accepted and not yet
  published is gone with the reset.
- `unusable` — a refused lock, an unresolvable path, damaged current-epoch
  bytes. **Do not delete the store.** A fresh file fixes none of these and
  destroys the only copy of unpublished writes. Check the path, permissions, and
  disk.
- `held-by-another-owner` — a daemon is already running for this home; stop it
  first.

The full reset is offered only for `superseded-epoch`. It stops and reaps the
selected instance under its startup lock, uses NMP's reset API for `nmp.redb`,
clears SQLite/session/attachment runtime, and leaves configuration and native
profiles intact. It refuses unsafe attachment targets such as root, HOME, or a
path overlapping configuration.

Never diagnose this by reading the refusal text. Mosaico branches on
`nmp::EngineError`; a message that says "predates the schema marker" once meant
both of the first two.

## Management key is not admin

Verify `userNsec` and `mosaicoPrivateKey` are distinct and that the relay-owner
human pubkey is the sole whitelist entry. Backend keys must not be added to the
human whitelist. Then confirm no stale profile connected first on a reused
relay. Prefer a fresh auto-selected port and freshly generated profile state.

## Model or provider arg rejected

Capture the provider's exact error. For direct mode, choose the cheapest model
the installed CLI accepts. For durable launch defaults, change the profile's
explicit `MOSAICO_DEV_*_ARGS_JSON` override and regenerate it. For a one-launch
override, pass flags after the profile to `scripts/launch-agent`, which inserts
`--`.

## Missing events

Check in order:

1. The launch accepted the prompt and produced a session id.
2. The generated config points to the current relay.
3. The sender used `--tag <target>`, not literal `@target` text.
4. Croissant logged the connection, subscription, and event.
5. `nak` reads the same relay URL.
6. Daemon logs show delivery or a concrete rejection.

## Cleanup order

```bash
skills/mosaico-dev/scripts/cleanup-lab "${LAB_ENV}"
```

The helper stops recorded containers before the relay PID. Preserve the work
directory when a failure still needs diagnosis.

## Final stale-surface audit

From the repository root, audit active skill files with `rg` for removed
product names, environment prefixes, config keys, and launch forms. Historical
wiki material is outside this skill cleanup unless explicitly in scope.

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

## Bundle or agent config is rejected

Validate the exact schema:

```bash
jq 'to_entries[] | {bundle:.key,keys:(.value|keys),harness:.value.harness,transport:.value.transport,args:(.value.args // [])}' \
  .container-state/<profile>/mosaico/harnesses.json
jq '{slug,harness,profile,perSessionKey,has_secret:has("secret_key"),has_public:has("public_key")}' \
  .container-state/<profile>/mosaico/agents/<slug>.json
```

Each bundle allows only `harness`, `transport`, and optional string-array
`args`. The agent owns optional string `profile`. A `perSessionKey: true` agent
must be keyless. Regenerate invalid state; do not add aliases or duplicate old
fields.

## Launch rejects arguments

The current surface is:

```text
mosaico agents [TARGET] [PROMPT] [--workspace ...] [--channel [ROOM]] [--name ...]
```

Provider flags belong in bundle `args`, not after a separator or in launch-time
override flags. Regenerate the profile and call the helper with no trailing
launch args. Direct mode may still receive provider CLI args.

## Launch target is missing or ambiguous

```bash
bash containers/mosaico/run --profile <profile> mosaico agents
```

A non-interactive run prints available targets. Check live harness detection,
configured agents, and the installed global/workspace native agent directories.
`harnesses.json` is launch policy, not catalog membership; a missing compatible
bundle is created on first realization. If the same native slug exists in several
harnesses, use the harness-suffixed target printed by the inventory.

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

Doctor/install must use the real harness name (`claude-code`, `codex`, or
`opencode`) even when the public agent slug differs.

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

The default reset removes both stores. If manually repairing an already-stopped
disposable profile, target only that profile:

```bash
rm -f .container-state/<profile>/mosaico/daemon.sock
rm -f .container-state/<profile>/mosaico/state.db \
  .container-state/<profile>/mosaico/state.db-shm \
  .container-state/<profile>/mosaico/state.db-wal
rm -f .container-state/<profile>/mosaico/nmp.redb
```

Never delete these from a live or non-disposable profile without explicit
authorization.

## Management key is not admin

Verify `userNsec` and `mosaicoPrivateKey` are distinct and that the relay-owner
human pubkey is the sole whitelist entry. Backend keys must not be added to the
human whitelist. Then confirm no stale profile connected first on a reused
relay. Prefer a fresh auto-selected port and freshly generated profile state.

## Model or provider arg rejected

Capture the provider's exact error. For direct mode, choose the cheapest model
the installed CLI accepts. For launch mode, change the profile's explicit
`MOSAICO_DEV_*_ARGS_JSON` override and regenerate it. Do not append flags to
`mosaico agents`.

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

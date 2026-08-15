# Lab start: image, relay, profiles

## Image and doctor

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

## Relay

```bash
skills/mosaico-dev/scripts/start-croissant-relay
```

For a multi-human run, name the isolated human identities up front:

```bash
MOSAICO_DEV_HUMAN_NAMES_JSON='["Pablo","Alice","Bob"]' \
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

The helper chooses an unused high port by default, launches the external
Croissant executable selected by `MOSAICO_DEV_CROISSANT_BIN`,
`NIP29_RELAY_BIN`, or PATH, binds it to the Apple container bridge, waits for
NIP-11, and records the relay owner identity without printing its secret. A
fresh port prevents stale agents from older runs from claiming the new
workspace first. Pin `MOSAICO_DEV_RELAY_PORT` only when shared port behavior is
itself under test.

## Profiles

Single profile:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" claude-acp
```

Multi-provider lab:

```bash
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" \
  claude claude-acp codex codex-app-server grok goose goose-acp hermes hermes-acp \
  kimi kimi-acp opencode opencode-acp
```

Each profile receives:

```text
.container-state/<profile>/mosaico/config.json
.container-state/<profile>/mosaico/presets.json
.container-state/<profile>/mosaico/agents/<slug>.json
```

The writer resets profile-local Mosaico state by default, including `state.db*`,
the daemon socket/logs, sessions, and `nmp.redb`. It preserves provider home and
build caches.

The device config uses the relay owner as `userNsec`, includes every generated
human pubkey in `whitelistedPubkeys`, and uses a distinct per-profile backend
key as `mosaicoPrivateKey`. Generated per-session agent files are keyless.
Inspect the public shape:

```bash
jq '{relays,indexerRelay,backendName,whitelistedPubkeys}' \
  .container-state/claude-acp/mosaico/config.json
jq . .container-state/claude-acp/mosaico/presets.json
jq '{slug,harness,preset,profile,perSessionKey,has_secret:has("secret_key"),has_public:has("public_key")}' \
  .container-state/claude-acp/mosaico/agents/claude.json
```

Do not print `userNsec`, `mosaicoPrivateKey`, or provider auth files.

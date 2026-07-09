# tenex-edge end-to-end harness

A reusable local rig that boots a real Nostr relay and **two fully isolated
tenex-edge backends** against it, then proves they coordinate *through the
relay* using existing functionality. Built to be extended for the upcoming
"subgroup task rooms" feature.

```
        ws://127.0.0.1:10547
   ┌──────── NIP-29 relay ──────────┐
   │  (NIP-29 groups, relay-signed    │
   │   39000/39001/39002 metadata)    │
   └────▲───────────────────▲─────────┘
        │ 9007/9002/9000     │ reads 39000
   ┌────┴─────┐         ┌────┴─────┐
   │ backend  │         │ backend  │   each: own daemon, socket,
   │  edge-a  │         │  edge-b  │   state.db, keystore, config.
   └──────────┘         └──────────┘   NO shared filesystem state.
```

## TL;DR

```bash
cargo build                 # build the tenex-edge binary under test
./e2e/run.sh                # boot relay + 2 backends, run the smoke test
./e2e/teardown.sh           # stop everything, wipe scratch state
```

`run.sh` is hermetic: it tears down any prior run, starts a fresh relay with
empty data, and exits non-zero with a clear `FAIL` line if anything is wrong.

## The Relay

The default local relay binary is [`croissant`](https://viewsource.win) (at
`/tmp/croissant-smallmap` when present, else `${HOME}/Work/croissant`), a Go
relay built on `khatru` + the `fiatjaf.com/nostr/nip29` library.

**It fully implements NIP-29 relay-based groups.** Evidence:

- `relay.go`: `relay.Info.AddSupportedNIP(29)` — advertised in its NIP-11 doc
  (the rig hard-asserts this on boot).
- `reject_event.go` / `process_event.go`: handles client-published
  **9007** create-group, **9000** put-user, **9001** remove-user, **9002**
  edit-metadata, plus 9021/9022 join/leave; the relay itself **signs and
  broadcasts** the replaceable **39000** (metadata), **39001** (admins),
  **39002** (members) state events with its own key.
- `subgroup_test.go`: it even carries the **NIP-29 subgroup extension** — a
  `["parent", <group-id>]` tag on the 9007 create that is re-emitted on 39000,
  with create-time validation (parent must exist, creator must be a parent
  admin, no cycles). This is exactly the substrate the subgroup-task-rooms
  feature will build on.

### Config that matters for the rig

- **Env:** `PORT` (default 9888; rig uses 10547), `HOST` (127.0.0.1),
  `DATAPATH` (absolute, per-run scratch dir), `OWNER_PUBLIC_KEY` (required hex;
  rig uses edge-a's pubkey), `DOMAIN` (empty for local → plain `http`/`ws`).
- **Relay key:** auto-generated on first boot into `<DATAPATH>/settings.json`
  as `relay_secret_key`; it signs the 39000/39001/39002 events.
- **No NIP-42 AUTH needed for our flow.** Event *publishing* is validated by
  `event.PubKey` against NIP-29 group admin/membership rules, not by AUTH. AUTH
  is only required to (a) *read* a **private** group, or (b) access gift-wraps.
  The rig creates **closed + public** groups, which are readable by anyone — so
  backend-b can subscribe and read without authenticating. (The HTTP NIP-98
  cookie in `global/auth.go` is only for the web UI / settings page.)
- **Presence gate is off by default.** `hasPresence()` returns `true` when no
  presence relays are configured (the default), so group creation and joins are
  not gated on a kind:0 living elsewhere.

## The two backends

tenex-edge keeps all writable state under `TENEX_EDGE_HOME` (default
`~/.tenex/edge`): the daemon **socket, lock, log, and `state.db`** all live
there (`src/daemon/mod.rs`). It reads device config from `TENEX_CONFIG`
(default `~/.tenex/config.json`) and treats `TENEX_DIR` as the shared `~/.tenex`.

The rig gives each backend its own trio of these, under
`$E2E_WORK/<name>/`, so the two daemons are completely independent:

| backend | `TENEX_EDGE_HOME`            | `TENEX_CONFIG`                 | identity key      |
|---------|-----------------------------|--------------------------------|-------------------|
| edge-a  | `$E2E_WORK/edge-a/edge`     | `$E2E_WORK/edge-a/config.json` | minted, cached    |
| edge-b  | `$E2E_WORK/edge-b/edge`     | `$E2E_WORK/edge-b/config.json` | minted, cached    |

Each `config.json`:

```json
{
  "whitelistedPubkeys": ["<edge-a-pub>", "<edge-b-pub>"],
  "relays": ["ws://127.0.0.1:10547"],
  "indexerRelay": "ws://127.0.0.1:10547",
  "backendName": "edge-a",
  "userNsec": "<this backend's seckey>",
  "tenexPrivateKey": "<this backend's seckey>"
}
```

Both backends whitelist **both** pubkeys, so each is a trusted admin on every
group. `tenexPrivateKey` is the backend's own identity — its pubkey is added as
a group admin by `open_project`.

The daemon is **auto-spawned** on the first client call (`spawn_daemon_if_absent`)
and **inherits the client's environment**, so the isolation env vars propagate
to it automatically. The rig's `edge()` helper scrubs any inherited
`$TENEX_EDGE_BIN` so the daemon re-execs the binary under test, not a
dev-shell-installed one.

## What the smoke test does

The production trigger for group creation is **`session_start`**: when a session
starts in a project directory, the daemon's provider runs `open_project`, which
publishes **9007 create-group → 9002 lock (closed/public) → 9000 put-user** for
the backend's admins and the session agent (`src/fabric/provider.rs`).

1. **edge-a** drives a `claude-code` `session-start` hook in a project dir
   registered in `~/.tenex/edge/projects.json` with the slug `e2e-demo` (via
   `tenex-edge project init`):
   ```bash
   echo '{"session_id":"…","cwd":"…/work/e2e-demo"}' \
     | TENEX_EDGE_AGENT=claude tenex-edge harness hook claude-code --type session-start
   ```
   → daemon-a creates the NIP-29 group `e2e-demo` on the relay.
2. **Direct relay check:** `nak req -k 39000 -d e2e-demo ws://127.0.0.1:10547`
   returns the relay-signed metadata event → the group really landed on the relay.
3. **edge-b** (a separate install, separate daemon + db) runs `project list`:
   ```bash
   tenex-edge project list      # → e2e-demo
   ```
   Backend-b learning the group exists is only possible via the shared relay;
   the backends share no filesystem state. That is the proof of cross-backend
   communication.

Expected `run.sh` tail:

```
ok  relay up; NIP-11 advertises NIP-29
nip29: group not found; publishing kind:9007 create-group
nip29: create-group accepted or already existed
nip29: group lock accepted or already existed
nip29: admin grant accepted for <edge-b>
nip29: agent membership accepted for <session>
ok  backend-a project list shows 'e2e-demo'
ok  relay holds kind:39000 metadata for 'e2e-demo'
ok  PASS — backend-b observed backend-a's group 'e2e-demo' through ws://127.0.0.1:10547
```

> An `admin grant rejected for <edge-a>` line is normal: edge-a's key is also the
> relay's `OWNER_PUBLIC_KEY`, so it is already implicitly admin and the redundant
> grant is a no-op.

## Tunables (env, set at the top of your shell or inline)

| var                    | default                                   | meaning |
|------------------------|-------------------------------------------|---------|
| `RELAY_PORT`           | `10547`                                   | relay ws port |
| `RELAY_HOST`           | `127.0.0.1`                               | relay bind host |
| `E2E_PROJECT`          | `e2e-demo`                                | project slug / group id driven by the test |
| `E2E_WORK`             | `$TMPDIR/tenex-edge-e2e`                  | scratch root (relay data, backend homes, logs) |
| `E2E_TENEX_EDGE_BIN`   | `<repo>/target/debug/tenex-edge`          | binary under test (override only via THIS var) |
| `NIP29_RELAY_DIR`      | `/tmp/croissant-smallmap` if present, else `$HOME/Work/croissant` | NIP-29 relay checkout |
| `NIP29_RELAY_BIN`      | `$NIP29_RELAY_DIR/croissant`              | NIP-29 relay binary |
| `TENEX_EDGE_DEBUG`     | `1`                                       | verbose daemon logging |

> **Do not** override the binary with `$TENEX_EDGE_BIN` — tenex-edge itself reads
> that as a daemon-spawn override and it is commonly exported in a dev shell.
> Use `E2E_TENEX_EDGE_BIN`.

## Inspecting / extending

Reuse the `edge()` helper from `lib.sh` for any backend command:

```bash
source e2e/lib.sh
edge edge-b project list
edge edge-b who --all-projects
edge edge-a chat write --message 'hello from a' --project e2e-demo
nak req -k 39000 "$RELAY_WS"          # all group metadata on the relay
nak req -k 9      -h e2e-demo "$RELAY_WS"   # chat messages in the group
```

Logs: `$E2E_WORK/relay.log`, `$(backend_edge_home edge-a)/daemon.log`,
`$(backend_edge_home edge-b)/daemon.log`.

**To extend for subgroup task rooms:** the NIP-29 relay already enforces the `parent`
tag rules, so a new test can have edge-a create a child group
(`["parent","e2e-demo"]` on the 9007) and assert edge-b sees the parent link on
the child's 39000. Add a new `run-*.sh` that sources `lib.sh` and reuses the
`edge()` / `wait_for` helpers.

## BDD scenario matrix

Use this matrix when validating the recent launch/session/fabric integration
PRs together. A scenario is complete only when the named evidence passes on the
current binary under test.

| id | Given | When | Then | Evidence |
|----|-------|------|------|----------|
| BDD-01 | a clean croissant relay and two isolated backends | backend-a starts a session in a project | the project group is created on the relay | `e2e/run.sh` |
| BDD-02 | backend-b shares only the relay with backend-a | backend-b lists projects | backend-b sees backend-a's project through relay state, not shared files | `e2e/run.sh` |
| BDD-03 | an existing project group and two backend admins | backend-a creates a child channel with agents on both backends | the child 39000 carries `parent=<project>` and 39001 includes both admins | `e2e/run-subgroup.sh` |
| BDD-04 | a relayed add-agents orchestration event names a role on backend-b | backend-b receives the kind:9 | backend-b mints the role and adds it as a child member | `e2e/run-subgroup.sh` |
| BDD-05 | two live sessions of the same agent in the same room | both session-start hooks run concurrently | the sessions get distinct ordinal labels and pubkeys | `e2e/run-ordinal.sh` |
| BDD-06 | the same ordinal is free in a different room | that agent starts in an alternate channel | the ordinal label and pubkey are reused across rooms | `e2e/run-ordinal.sh` |
| BDD-07 | two same-agent sessions are live in one room | one session mentions the other using `chat write --session` | the mention resolves to the recipient and the kind:9 carries both `#p` and `#h` | `e2e/run-ordinal.sh` |
| BDD-08 | a session already holds an ordinal in the destination room | `channels switch --session` would collide that ordinal | the daemon rejects the switch with `already active` | `e2e/run-ordinal.sh` |
| BDD-09 | a portable PTY supervisor is live | `pty list` resolves its metadata | the session is reported live | `e2e/run-pty.sh` |
| BDD-10 | an attach client connects to a live PTY socket | the supervisor has backlog output | the attach client receives the backlog and later fanout | `e2e/run-pty.sh` |
| BDD-11 | multiline text is piped to `pty inject --bracketed --no-submit` | a separate submit is sent | the PTY receives one bracketed multiline paste and submit | `e2e/run-pty.sh` |
| BDD-12 | multiline text is piped to `pty inject --no-submit` | a separate submit is sent | the PTY receives the plain multiline payload | `e2e/run-pty.sh` |
| BDD-13 | a live PTY session is attached | injected input is echoed by the terminal | the attach client sees the live echo | `e2e/run-pty.sh` |
| BDD-14 | a PTY session is live | `pty resize` and `pty kill` are issued | resize succeeds, the child exits, and metadata is removed | `e2e/run-pty.sh` |
| BDD-15 | a local agent has a harmless configured command | `tenex-edge launch <agent>` runs without a TTY | launch creates a live portable PTY session that accepts inject and kill | `e2e/run-pty.sh` |
| BDD-16 | a launched PTY-backed session has a `pty_session` alias | a user-authored kind:9 mentions that session | the daemon injects the message into the running PTY | `cargo test --test daemon_integration operator_kind9_injects_into_running_launch_session -- --test-threads=1` |
| BDD-17 | a user-authored kind:9 mentions an offline local agent identity | the agent is available locally | the daemon spawns a PTY-backed session and injects the triggering message | `cargo test --test daemon_integration operator_kind9_to_offline_local_agent_spawns_and_injects -- --test-threads=1` |
| BDD-18 | validation targets reference PTY aliases and session surfaces | `tenex-edge debug validate` renders the target | evidence uses `pty_session:<id>` and reports exact proof boundaries | `cargo test --lib probe validate_render` |
| BDD-19 | exact session targeting is needed for chat/channel operations | `--session <session-id>` is supplied | the requested session anchor wins over ambient environment hints | `cargo test chat_write_accepts_explicit_session_anchor channels_switch_accepts_explicit_session_anchor` |
| BDD-20 | backend-addressed management commands arrive as p-tagged kind:9 events | add/list/kill/archive commands are parsed | the daemon routes them through the management-command handler | `cargo test daemon::server::management_command` |
| BDD-21 | hosted-session transport has moved to portable PTY | the tree is searched for replaced transport vocabulary | no current source, docs, tests, or filenames retain the replaced host path | `git grep -n -i <old-term> HEAD` plus filename search |
| BDD-22 | a peer's kind:0 lives only on the relay and it is added to the project as a member | the daemon receives the relay-signed kind:39002 | it proactively fetches the peer's kind:0 and `who` renders the peer by NAME, not hex, with no explicit warm | `e2e/run-warm.sh` |
| BDD-23 | the daemon's management key is a channel admin | `who --all-projects` renders the roster | the management pubkey is excluded from the member list | `e2e/run-warm.sh` |

## Files

- `lib.sh` — shared config, paths, key minting, the `edge()` / `wait_for` helpers.
- `run.sh` — boot + smoke test (idempotent; tears down first).
- `run-subgroup.sh` — child channel orchestration across two isolated backends.
- `run-ordinal.sh` — same-agent ordinal identity, routing, and collision checks.
- `run-pty.sh` — portable PTY controls: attach protocol, multiline inject,
  `tenex-edge launch <agent>`, resize, kill, and metadata cleanup.
- `run-warm.sh` — proactive kind:0 profile warming (a relay-only peer resolves by
  name in `who` without any explicit warm) and backend-mgmt-key roster exclusion.
- `teardown.sh` — stop relay + daemons, reclaim the relay port, wipe scratch.

## Caveats

- macOS / `lsof` are assumed for port reclaiming.
- The default relay build is CGO (bleve/sqlite); first build ~1 min. The rig
  builds it once if the configured binary is missing.
- Each `run.sh` starts a **fresh** relay (empty data), so group state never
  carries across runs — every run exercises the create path from scratch.

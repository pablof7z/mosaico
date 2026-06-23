# tenex-edge: per-machine daemon design

Status: implemented. Implements the architecture change
from **per-session process** to **one per-machine daemon** that solely owns
`state.db`, the relay connection, chat delivery, presence, NIP-29 membership cache,
and peer pruning.

## 1. Why

Today every Claude Code / Codex / OpenCode session spawns its own detached
engine (`tenex-edge __run-session`), and **every** CLI invocation (`who`,
`chat`, `who`, `turn-start`, …) opens its **own** `rusqlite`
connection to the single SQLite file at `~/.tenex/edge/state.db`. Under ~16
concurrent per-session writers this corrupted `state.db` (a real incident).
Root cause: many independent processes each believe they own the db, and N
sessions also mean N independent relay connections.

The fix: collapse to **one daemon per machine** that is the sole owner of the
db and the (single) relay connection. Every CLI invocation and every
per-session engine becomes a **thin client** that talks to the daemon over a
Unix domain socket. One writer by construction → corruption window goes to
zero; N relay connections collapse to one.

This is a pure-internal change: **external CLI behavior and output are
preserved** (the Python hooks in `integrations/` and the parallel
Claude channel adapter shell out to these verbs and parse their stdout).

## 2. Stages (sequenced, each independently reviewable)

0. **Build/green baseline.** (Already green on disk; the broken state observed
   mid-work was a Syncthing sync race that resolved.) `cargo test` = 54 unit + 1
   e2e green.
1. **WAL stopgap** — `src/state.rs` only. `journal_mode=WAL` (already present),
   add `synchronous=NORMAL`, keep `busy_timeout=5000`. No FK pragma (no FK
   constraints in the schema). This is the bandage so the *still-running*
   multi-writer code stops corrupting during development. Stays (harmless) after
   the daemon is sole writer. **Done.**
2. **Daemon + single writer** — introduce the UDS daemon, spawn-if-absent,
   lock/socket/stale-reclaim/version-handshake, and the RPC protocol. CLI verbs
   become thin RPC clients. The daemon owns one `Store`. **Done.**
3. **Engine + relay relocation** — move the per-session engine
   (`runtime::run_session`) **into** the daemon as a per-session async task, and
   collapse the relay connection(s) to **one** shared `Transport` inside the
   daemon. `__run-session` (the detached subprocess) is removed; `session_start`
   is a daemon RPC that spawns an in-process `SessionTask`. **Done.**

## 3. Process model

```
              ┌──────────────────────────── machine ────────────────────────────┐
              │                                                                   │
  hook /      │   ┌─────────────┐   UDS    ┌──────────────────────────────────┐  │
  CLI    ───▶ │   │ thin client │ ───────▶ │  tenex-edge daemon (single proc) │  │
 (one-shot)   │   │ (CLI verb)  │ ◀─────── │                                  │  │
              │   └─────────────┘  JSON    │  • owns state.db (one Store)     │  │   one
              │                            │  • owns ONE relay Transport ─────┼──┼──▶ relay
              │                            │  • per-session async tasks       │  │  (NIP-42)
              │                            │  • chat / presence / pruning     │  │
              │                            │  • NIP-29 membership cache       │  │
              │                            └──────────────────────────────────┘  │
              └───────────────────────────────────────────────────────────────────┘
```

- The daemon is a normal `tenex-edge` invocation: `tenex-edge __daemon` (hidden
  subcommand, like today's `__run-session`).
- The daemon runs the **tokio multi-thread runtime** (already how `main` builds
  it). It holds exactly one `Store` (single SQLite connection → one writer by
  construction) and one `Transport` (one relay connection).
- Per-session work that today lives in a detached process becomes a tokio task
  inside the daemon (`SessionTask`), keyed by `session_id`.

### Spawn-if-absent

Any invocation that needs state does `Daemon::connect_or_spawn()`:

1. Try to `connect()` to `$TENEX_EDGE_HOME/daemon.sock` (default base
   `~/.tenex/edge`).
2. If that succeeds → return the client.
3. If it fails (no listener, or stale socket) → acquire the startup lock (§4),
   re-check (another racer may have just bound), and if still absent, **spawn**
   the daemon (double-fork / `setsid`, detach stdio → `~/.tenex/edge/daemon.log`),
   then poll-connect with a short timeout.

### Idle exit

The daemon **idle-exits** when no sessions are alive. It reuses the existing
liveness/heartbeat machinery:

- A session is "alive" when its `SessionTask` is running (it ends on `watch_pid`
  death, `session-end` RPC, or SIGTERM-equivalent intent).
- When the alive-session count drops to zero, start a **grace timer**
  (`TENEX_EDGE_DAEMON_GRACE_S`, default 120s). If no new `session-start` /
  client connection arrives before it fires, the daemon shuts down cleanly
  (publishes idle/expired presence for any lingering state, drops the socket,
  releases the lock).
- Any inbound client connection or `session-start` cancels the grace timer.

This keeps the daemon from outliving the fabric while avoiding flapping when
sessions briefly come and go.

## 4. Socket, lock, stale-reclaim, version handshake

Files (all under `$TENEX_EDGE_HOME`, default `~/.tenex/edge`):

| file          | role                                                        |
|---------------|-------------------------------------------------------------|
| `daemon.sock` | the UDS the daemon binds; clients connect here              |
| `daemon.lock` | `flock`'d during startup to serialize racing spawners       |
| `daemon.log`  | detached daemon stdout+stderr                               |
| `state.db`    | the one SQLite db, owned solely by the daemon               |

### Race-safe startup

```
connect_or_spawn():
  if connect(sock) ok: return client
  lock = open("daemon.lock"); flock(lock, LOCK_EX)        # winner proceeds, losers wait
  if connect(sock) ok: unlock; return client              # someone bound while we waited
  if sock path exists:                                    # stale-socket case
      # a file is present but connect refused → previous daemon died uncleanly
      unlink(sock)                                         # safe: we hold the lock
  spawn_detached_daemon()                                  # binds sock under the same lock discipline
  unlock(lock)                                             # released AFTER spawn returns; daemon re-flocks on its own
  poll_connect(sock, deadline=~3s): return client
```

The **daemon** itself, on startup, also `flock`s `daemon.lock`, unlinks any
stale `daemon.sock`, binds, and only then begins serving — so even if two
daemons are spawned by a pathological race, only one can bind. The lock is held
by the daemon for its whole lifetime (advisory; cheap), which doubles as a
"is a daemon running" probe.

### Stale-socket case

Socket file exists but `connect()` is refused (ECONNREFUSED / ENOENT on the
peer) → the previous daemon crashed without cleaning up. Resolution: under the
lock, `unlink` the socket and reclaim. (Bind would otherwise fail with
EADDRINUSE.)

### Protocol version + version-skew handshake

The first line a client sends and the first line the daemon replies are a
**handshake** carrying a `protocol` integer:

```jsonc
// client → daemon, first frame
{"hello": {"protocol": 3, "client_version": "0.1.0"}}
// daemon → client, first frame
{"welcome": {"protocol": 3, "daemon_version": "0.1.0"}}
```

- `PROTOCOL_VERSION` is a `const u32` bumped on any breaking RPC change.
- **Newer client, older daemon** (binary upgraded under a running daemon — the
  human cutover): the client sees `welcome.protocol < its own` and sends a
  `{"please_exit": {"protocol": <new>}}` control frame. The old daemon, on
  seeing a request protocol it doesn't understand, replies
  `{"error": {"code": "protocol_skew", ...}}`, finishes draining, and exits
  (releasing the socket/lock). The client then loops back into
  `connect_or_spawn()`, which now spawns the **new** binary's daemon. Net: a
  newer client transparently re-execs the daemon to its own version rather than
  speaking a stale protocol.
- **Older client, newer daemon**: the client refuses (prints a clear "restart
  your session / reinstall" error). This is the rarer direction and not silently
  bridged.

Rationale: we never want a stale-protocol conversation to half-succeed. A
single version int + an exit-and-respawn handshake is the minimum that makes the
human cutover (drop a new binary in place, restart sessions) safe even while an
old daemon is live.

## 5. Wire protocol

Newline-delimited JSON over the UDS (one JSON object per line). Chosen over
length-prefixed for debuggability (`socat` / `nc` can talk to it) and because
all payloads are small. Each request is one line; each response is one or more
lines (see streaming, below) terminated by an `end` frame.

### Frame shapes

```jsonc
// request
{"id": 1, "method": "who", "params": { ... }}
// single response
{"id": 1, "ok": { ... }}
{"id": 1, "error": {"code": "...", "message": "..."}}
// streaming response (N frames then an end marker)
{"id": 1, "item": { ... }}
{"id": 1, "item": { ... }}
{"id": 1, "end": true}
```

`id` correlates responses to requests (allows pipelining; in practice each thin
CLI client issues one request and exits). The **streaming** shape (`item`* then
`end`) is built in from the start — `tail` needs server-push, and the
future `subscribe --json` verb the host adapters will want is the same
machinery. One-shot verbs simply emit a single `ok` and no `item`/`end`.

### Why the streaming framing matters (the key design call)

Walking each verb's true I/O shape:

| verb               | shape                | mechanism                                            |
|--------------------|----------------------|------------------------------------------------------|
| `session_start`    | one-shot             | daemon spawns a `SessionTask`, returns `session_id`  |
| `session_end`      | one-shot             | daemon stops the `SessionTask`                       |
| `turn_start`       | one-shot (may be big)| daemon drains chat + builds context, returns text    |
| `turn_check`       | one-shot, read-only  | peek chat; no writes                                 |
| `turn_end`         | one-shot             | flip turn state                                      |
| `chat_write`       | one-shot             | daemon publishes kind:9 chat event on the relay      |
| `chat_read`        | one-shot             | daemon returns chat history for the session/project  |
| `who`              | one-shot             | snapshot rows                                        |
| `who --live`       | client-side poll     | client calls `who` each refresh; renders terminal    |
| `doctor`           | one-shot             | daemon does the relay round-trip, returns result     |
| `tail`             | **stream**           | daemon pushes decoded fabric events until disconnect |

`tail` forces the protocol beyond simple req/resp: the client cannot open its own relay
  subscription anymore (the daemon owns the single relay connection), so the
  daemon must stream decoded events to the client as they arrive, indefinitely,
  until the client disconnects (Ctrl-C). This is what makes the `item`*/`end`
  streaming shape mandatory rather than optional.

Client disconnect (EOF / broken pipe on the UDS) is the universal cancel signal:
it drops a `tail` subscription forwarder.

## 6. RPC surface (every method)

Coarse, lifecycle/intent-level — **not** fine-grained DB ops. The engine lives
inside the daemon, so there is no per-DB-op RPC chatter; the surface is
low-frequency lifecycle signals from hooks, CLI reads, and chat writes.

All params/results are JSON. `session` fields are full session ids; the daemon
resolves "my session from cwd/env" the same way the CLI does today
(`resolve_session`: explicit → `$TENEX_EDGE_SESSION` → latest-alive-for-project).
**The resolution stays daemon-side** so behavior is identical; the client passes
the explicit `--session` if any, plus its `cwd` and the `TENEX_EDGE_SESSION`
env value, and the daemon resolves.

### `session_start`
Spawns an in-daemon `SessionTask` (publishes profile, presence, subscribes,
distills, routes mentions — today's `runtime::run_session`).
```jsonc
params: {"agent": "coder", "session_id": "te-…"|null, "cwd": "/path", "watch_pid": 12345|null}
result: {"session_id": "te-…", "codename": "bravo4217"}   // session_id printed verbatim to stdout
```
The `codename` is the human-friendly session label (NATO phonetic word + 4-digit
number, e.g. `bravo4217`, `echo0163`), produced by `session_codename` in util.rs. It
is a display/addressing convenience only — the space is 26×10000 = 260000 codenames,
so it is not collision-free at scale and is never used as identity.
The provider opens the project's NIP-29 group and adds the session agent as a
relay member before the engine publishes presence. There is no local agent
allow/block file in the NIP-29 path.

### `session_end`
```jsonc
params: {"session": "te-…"}
result: {"ended": true|false}    // false ⇒ no such session
```
Stops the `SessionTask` (which publishes idle presence/status and marks the
session dead). stderr message (`session … ended` / `no such session: …`) is
produced client-side to match today's output.


### `who`
```jsonc
params: {"project": "…"|null, "all": bool, "cwd": "/path"}
result: {"now": u64, "rows": [ {source, fresh, slug, project, status, host,
                                session_id, age_secs}, … ]}
```
Returns the `WhoSnapshot` rows (serializable mirror). **All rendering stays
client-side** — `render_who_once` (colored), `render_who_plain`, and the
`--live` terminal UI are unchanged and consume these rows. `who --live` just
re-issues `who` each refresh tick (no streaming).

### `turn_start`
```jsonc
params: {"session": "te-…", "transcript": "/path"|null, "json": bool, "cwd": "/path"}
result: {"context": "…"|null}    // the assembled injection text, or null
```
Daemon does everything `turn_start` does today (mark turn, set transcript,
drain pending chat messages, full roster on first turn / deltas after). Client
emits via `emit_context` (plain or `{"systemMessage":…}`) to keep byte-identical
output. Empty session id ⇒ no-op (returns `context: null`).

### `turn_check`
```jsonc
params: {"session": "te-…"|null, "json": bool, "cwd": "/path", "env_session": "…"|null}
result: {"context": "…"|null}    // peek only; no chat drain, no writes
```

### `turn_end`
```jsonc
params: {"session": "te-…"}
result: {"ok": true}
```

### `doctor`
```jsonc
params: {}
result: {"relays": [...], "probe_pubkey": "hex", "publish": "OK …"|"ERR …",
         "readback": "N event(s) …"|"ERR …"}
```
Daemon performs the publish + read-back on the shared relay; client prints the
existing multi-line report.

### `tail` (streaming)
```jsonc
params: {"project": "…"|null}
stream: {"item": {"line": "<rendered fabric line>"}}   // repeated
        … until client disconnects (Ctrl-C)
```
Daemon registers a forwarder on its shared relay subscription, decodes each
event with the codec, renders with the existing `render()` and streams the line.
The client just prints each `item.line`. (The daemon may need a project-scoped
ephemeral subscription distinct from its trusted-author subscription; it can add
a tail-scoped REQ for the duration of the connection.)

### `chat_read` (streaming)
```jsonc
params: {"session": "te-…"|null, "cwd": "/path", "env_session": "…"|null}
stream: {"item": { message fields }}
```
Streams unread chat-inbox messages for the session; used by the TUI reader.

### `chat_write`
```jsonc
params: {"session": "te-…"|null, "cwd": "/path", "message": "…", "mention": "…"|null, ...}
result: {"ok": true}
```
Publishes a chat message (NIP-C7 kind:9 event) from the session agent, optionally mentioning a specific peer session.

### `propose`
```jsonc
params: {"title": "…", "body": "…", "session": "te-…"|null, "cwd": "/path", ...}
result: {"event_id": "hex"}
```
Publishes a NIP-29 proposal (structured suggestion) to the project group.


### `project_list`
```jsonc
params: {}
result: {"projects": [ {slug, name, about, relay}, … ]}
```
Returns all known NIP-29 projects (group metadata) from the daemon's cache.

### `project_edit`
```jsonc
params: {"project": "…", "name": "…"|null, "about": "…"|null}
result: {"ok": true}
```
Publishes an updated NIP-29 kind:39000 group metadata event for the project.

### `project_members`
```jsonc
params: {"project": "…"}
result: {"members": [ {pubkey, slug, role}, … ]}
```
Returns the current membership list for the given NIP-29 group.

### `project_add`
```jsonc
params: {"project": "…", "pubkey": "hex"|null, "agent": "slug"|null}
result: {"ok": true}
```
Adds a pubkey or agent to the project's NIP-29 group (admin-signed kind:9000 add event).

### `project_remove`
```jsonc
params: {"project": "…", "pubkey": "hex"}
result: {"ok": true}
```
Removes a pubkey from the project's NIP-29 group (admin-signed kind:9001 remove event).

### `groups_create`
```jsonc
params: {"slug": "…", "name": "…"|null, "about": "…"|null, "parent": "…"|null, "cwd": "/path", ...}
result: {"group_id": "…", "relay": "wss://…"}
```
Creates a new NIP-29 group (subgroup or top-level); publishes kind:9007 create event.

### `groups_list`
```jsonc
params: {"project": "…"|null, "cwd": "/path"}
result: {"groups": [ {group_id, name, about, parent, relay}, … ]}
```
Lists NIP-29 groups visible to the daemon, optionally scoped to a project.

### `publish_profile`
```jsonc
params: {"agent": "slug", "name": "…"|null, "about": "…"|null}
result: {"event_id": "hex"}
```
Force-publishes or updates the agent's kind:0 profile.

### `statusline`
```jsonc
params: {"session": "te-…"|null, "cwd": "/path", ...}
result: {"working": bool, "status": "…", "session_count": N, "member_count": N,
         "is_member": bool, "pending": N, "pending_chat": N}
```
Pure-read snapshot for the host statusline integration — no drain, no writes.

### `whoami`
```jsonc
params: {"session": "te-…"|null, "cwd": "/path", ...}
result: {"agent": "slug", "pubkey": "hex", "session_id": "te-…", "project": "…"}
```
Returns the resolved identity for the calling session.

### `list_threads`
```jsonc
params: {"project": "…"|null, "cwd": "/path"}
result: {"threads": [ {thread_id, subject, from_slug, created_at, unread}, … ]}
```
Lists proposal threads for the session.

### `messages`
```jsonc
params: {"thread_id": "…"}
result: {"messages": [ {from_slug, body, created_at, mention_event_id}, … ]}
```
Returns all messages in a thread.

### `thread_meta`
```jsonc
params: {"thread_id": "…"}
result: {"subject": "…", "participants": [...], "created_at": N}
```
Returns metadata for a specific thread without fetching its messages.

### `ping`
```jsonc
params: {}
result: {"pong": true}
```
Health-check / keep-alive.

### `tmux_status`
Returns live tmux session state (panes, windows) known to the daemon.

### `tmux_send`
Sends keystrokes or text to a tmux pane.

### `tmux_spawn`
Spawns a new tmux window/session for an agent, optionally pre-loading a message.

### `tmux_attach`
Returns the tmux target string needed to attach to a session's pane.

### `tmux_resume`
Reconstitutes a dead harness session in tmux (re-opens the agent in its worktree).

### `tmux_resumable`
Returns the list of sessions that can be resumed (have a dead tmux pane but live session row).

### Control / handshake (not user verbs)
- `hello` / `welcome` (§4)
- `please_exit` (version-skew re-exec, §4)

## 7. How the engine relocates inside the daemon

Today: `session-start` writes a session row then forks `tenex-edge
__run-session …`, a separate process that opens its own `Store` and `Transport`
and runs `runtime::run_session`.

After: `session_start` RPC → the daemon spawns a tokio task running (a refactor
of) `runtime::run_session`, but:

- It uses the daemon's **shared** `Store` (passed as `Arc<Mutex<Store>>` or via a
  single-owner actor — see §8) instead of opening its own.
- It uses the daemon's **shared** `Transport`. The daemon maintains **one** relay
  connection and **one** union subscription (trusted authors ∪ all live session
  owners, per-project as needed). Incoming relay events are demuxed once,
  daemon-side, and routed to the right session chat queue(s) — replacing today's
  per-engine `handle_incoming`. Mentions route via the existing
  `compute_targets` / `route_mention` logic over all alive sessions.
- Presence/status/activity publishing, heartbeats, turn-driven distillation, and
  `watch_pid` death detection all move into the per-session task, but publish
  through the shared `Transport`.
- Peer-staleness pruning becomes a single daemon-level periodic task (today each
  engine prunes; now once).
- `__run-session` (the subcommand and the detached-fork code in `session_start`)
  is removed. `detach()` moves to daemon spawning only.

`EngineParams` is reused largely as-is, minus `store_path` (the task gets the
shared store) and with the transport injected.

## 8. State ownership & the single-writer guarantee

The **single SQLite connection** lives in the daemon and is the sole writer by
construction — that is the whole point. Concurrency model inside the daemon:

- One `Store` behind the daemon's request loop. Two viable shapes:
  1. **`Arc<Mutex<Store>>`** shared by the request handlers and session tasks.
     Simple; rusqlite `Connection` is not `Sync` so a `Mutex` (or
     `tokio::sync::Mutex`) is required anyway. All db access is short.
  2. **DB actor task**: a single task owns the `Store`; everyone sends
     `oneshot`-replied commands over an `mpsc`. Cleaner serialization, no lock
     held across `.await`.

  Decision: start with **`std::sync::Mutex<Store>`** (lock only around the
  synchronous rusqlite calls, never across `.await`). It is the smallest change
  from today's code and already guarantees one writer. Revisit the actor shape
  only if lock contention shows up (unlikely at this call frequency).

- The relay `Transport` is shared (`Arc<Transport>`); its methods are already
  `&self` and internally synchronized by `nostr-sdk`.

Because there is exactly one process with exactly one connection, the
multi-writer corruption class is eliminated regardless of WAL (WAL stays as
defense-in-depth + a small perf win).

## 8a. Multi-agent on ONE connection (verified empirically)

The daemon hosts several agent identities at once (`claude@`, `codex@`,
`opencode@`, plus same-agent siblings — identity is `(agent, machine)`, so each
is a distinct pubkey). The whole premise of the migration is **one relay
connection** for all of them. Two facts had to hold for that to be correct;
both were probed against the live `relay.tenex.chat` (see
`tests/relay_probe.rs`, run with `--ignored`):

1. **Cross-pubkey delivery.** A connection authenticated (NIP-42) as agent A
   *does* receive events p-tagged to a different agent B. The relay does **not**
   scope REQ delivery to the connection's authed identity. → A single shared
   subscription (union of all hosted pubkeys / projects) delivers every hosted
   agent's mentions. ✅ `one_authed_conn_receives_mentions_to_other_pubkeys`
2. **Multi-key publish.** An event **pre-signed by B** can be published over the
   A-authed connection (`client.send_event(&signed_by_b)`) and lands under B's
   authorship. → The daemon signs each outgoing event with the *originating
   agent's* `Keys` and sends it over the one connection. ✅
   `one_conn_publishes_events_signed_by_multiple_keys`

**Transport change required.** `Transport::connect` binds one `Keys` as the
connection signer (used for AUTH — fine, AUTH identity is irrelevant to
delivery per fact 1). But `publish_builder` signs with that one signer, which is
wrong for a multi-agent daemon. Add:

```rust
// sign with a specific agent's keys, publish over the shared connection
pub async fn publish_signed(&self, builder: EventBuilder, keys: &Keys) -> Result<EventId>;
```

The daemon picks the AUTH identity once (any one hosted agent key, or a stable
daemon key) and then `publish_signed`s each event with its true author. The
codec/wire output is unchanged.

## 8b. Demux + routing for multiple local agents (correctness)

Today `handle_incoming` / `route_mention` assume a single `me`. Inside the
daemon, "me" becomes the **set** of hosted local agent pubkeys:

- `is_self` = `local_pubkeys.contains(event.pubkey)` — skip our own
  profile/presence/activity/status for **any** hosted key.
- A `Mention` routes by `m.to_pubkey`: find the hosted agent whose pubkey equals
  `to_pubkey`, then `compute_targets` over **that agent's** alive sessions only
  (never another agent's). Sibling fan-out (untargeted mention → all of that
  agent's sessions) is preserved.
- Profile/presence/status from peers (non-local pubkeys) update the directory as
  today.

Test (added): a chat mention to A must land only in A's chat queue, never B's; untargeted
chat to A fans out to all of A's sessions.

## 8c. Session reconciliation on daemon (re)start (correctness)

The version-skew handshake (§4) and idle-exit can stop a daemon while session
rows are still `alive=1` in the db, and the new daemon's in-process
`SessionTask`s don't exist yet. On startup the daemon **reconciles**: for each
`alive=1` session row,

- `watch_pid` set and `pid_alive(watch_pid)` → respawn a `SessionTask` for it;
- else → `mark_session_dead`.

Without this, `who` / presence would lie after every daemon restart. (Idle-exit
only fires at zero alive sessions, so it doesn't orphan; the skew re-exec can,
hence reconciliation.)

## 8d. Clientful-but-sessionless connections vs idle-exit (§3)

`tail` and `who --live` hold a client connection open
without owning a session. Decision: **an open client connection cancels the
idle-grace timer** (the daemon counts "live sessions + open client connections"
for liveness). So a lone `tail` keeps the daemon up; when it disconnects and no
sessions remain, the grace timer starts. This avoids live readers being silently
killed by an idle-exit mid-stream.

## 8e. Working directory in presence/status + the `who` format  (IMPLEMENTED)

> **Implemented.** The user explicitly authorized this change (overriding the
> earlier byte-identical-`who` guardrail). The wire field, the `peer_sessions` +
> `sessions` `rel_cwd` columns, the daemon-side `remote` computation, and the new
> two-line `who` renderer are all in the codebase and live.
>
> **`who` stdout contract changed.** `who` now prints TWO lines per agent
> (`agent@project [session <id>] [<rel_cwd>]` then an indented status line) plus
> a ` (remote)` tag for genuinely-remote peers. Anything that parses `who` stdout
> (e.g. the parallel channel adapter / anything in `integrations/`, which is out
> of scope and untouched here) may need updating to the new format.
>
> **Wire field name:** `["rel-cwd", <rel>]` (omitted when empty/root). Decode
> tolerates its absence (old peers → `""`), so it is backward compatible.
>
> **Worktree caveat:** `rel_cwd` is computed relative to `project::project_root`
> (the dir holding the nearest `.tenex/project.json`, else `git rev-parse
> --show-toplevel`). For real `git worktree add` dirs, `--show-toplevel` returns
> each worktree's OWN path, so two worktrees both resolve to `.` and render
> bracket-less. To make `worktree1`/`worktree2` render distinctly, place a
> `.tenex/project.json` at their common parent so `project_root` resolves there.

Agents may run in different working dirs / git worktrees on the same machine
(`$PROJECT/worktree1` vs `worktree2`). Peers must see *where* a peer is working
so they don't fear colliding. This is additive: one new field on the
presence/status event + the peer state + the `who` renderer.

**Wire field — `rel_cwd` (relative cwd), not absolute.** Presence/status are
**public** kinds on `relay.tenex.chat` (world-readable). Broadcasting an
absolute `$HOME/...` path leaks the filesystem layout. So the engine computes
the cwd **relative to the project root** and publishes only that:

- `rel_cwd = cwd.strip_prefix(project_root)`, e.g. `worktree1`, `sub/dir`.
- cwd == project root → `"."` (rendered as omitted/`.`).
- can't resolve a project base → fall back to the cwd **basename** (still not
  the absolute path); absolute only as a last resort if even basename is empty.

`project_root` is the dir `project::resolve` walked up from (the nearest
ancestor holding the project marker), available at `session-start`. The
`Presence` domain struct and the `agent_status`/peer-session state gain a
`rel_cwd: String` field; `Status` carries it too so mid-turn `who` reflects it.
Peer state (`peer_sessions`) gains a `rel_cwd` column (migration: `ALTER TABLE
… ADD COLUMN rel_cwd TEXT NOT NULL DEFAULT ''`).

**New `who` line format:**

```
agent@project [session <id>] [<rel_cwd>]
    <current status / doing>
```

- `rel_cwd` shown in brackets only when non-empty and not `.`.
- **Host annotation:** if the peer is on the **same machine** as the viewer,
  show **no** host annotation (drop today's `@<host>` and `(this machine)` for
  local agents — local fleet renders clean). If the peer is on a **different**
  host, annotate `(remote)`. ("Same machine" = peer's `host` equals our
  `config::hostname()`/`host` label.)
- This replaces the current `slug@host — status  project  session <id>
  (this machine)/(Ns ago)` line in both `render_who_once` and `render_who_plain`,
  and the `--live` table gains a `WHERE`/`DIR` column. Freshness dot (●/○),
  staleness, own-fleet, and owner-scoping behavior are unchanged.

The `who` RPC result rows (§6) gain `rel_cwd: String` and a `remote: bool`
(computed daemon-side by comparing the peer host to the daemon's host), so all
rendering stays client-side.

## 9. Landmines preserved (must not regress)

- **rustls ring CryptoProvider** install in `main()` stays — the daemon is now
  the process that touches the relay, so the install must run on its path too
  (it already runs in `main` before dispatch; `__daemon` goes through `main`).
- **Identical standard-Nostr wire output** — the codec seam
  (`codec::kind1`) is untouched; the daemon publishes the same builders.
- **Relay NIP-42 AUTH warm-up fetch** before any subscribe — `Transport::connect`
  already does the `kind:0 limit 1` warm-up; the daemon connects once and that
  warm-up runs once, before its union subscription.
- **NIP-29 membership semantics** — group creation, owner admin backfill, and
  agent member admission remain provider-owned and relay-authoritative. Local
  allow/block files are not part of the active NIP-29 path.

## 10. Tests

- **Daemon spawn race**: N threads call `connect_or_spawn()` simultaneously;
  assert exactly one daemon binds and all clients connect.
- **Stale-socket reclaim**: create a `daemon.sock` file with no listener; assert
  `connect_or_spawn()` unlinks + reclaims and connects.
- **RPC round-trip**: start a daemon (test home), issue `who` / `chat_write` /
  `chat_read` RPCs, assert results.
- **Version-skew handshake**: client with `protocol = N+1` against a daemon
  pinned to `N` → daemon exits, client respawns and succeeds; assert the old
  daemon process is gone.
- **Concurrency / corruption repro**: ~16 concurrent clients issue writes via
  the RPC path; assert a single writer (one daemon pid) and `PRAGMA
  integrity_check = ok` afterward. Keep a direct-`Store` stress test as the
  original-corruption regression repro so the thing we're fixing stays asserted.
- Keep the existing 54 unit + the live-relay e2e (`nak serve`) green.

## 11. Follow-ups (out of scope here, noted for the host adapters)

- `subscribe --json` streaming verb (same `item`*/`end` machinery as `tail`,
  but emitting structured JSON events instead of rendered lines) — the channel
  adapters will want it.
- Optional: a `daemon-status` / `doctor --daemon` verb to report daemon pid,
  uptime, alive-session count.

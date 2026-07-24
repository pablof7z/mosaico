# mosaico: per-machine daemon design

Status: implemented. Implements the architecture change
from **per-session process** to **one per-machine daemon** that solely owns
`state.db`, NMP acquisition and durable group publication, chat delivery, presence,
NIP-29 membership cache, and peer pruning.

## 1. Why

The earlier per-session process model let every agent session and CLI invocation
open its own `rusqlite` connection to `~/.mosaico/state.db`. Under about 16
concurrent writers this corrupted `state.db` in a real incident. The root cause
was multiple independent processes treating the same database as theirs to own,
alongside one relay stack per session.

The fix: collapse to **one daemon per machine** that is the sole owner of the
database and relay-facing clients. Every CLI invocation and every
per-session engine becomes a **thin client** that talks to the daemon over a
Unix domain socket. One writer by construction → corruption window goes to
zero; N per-session network stacks collapse to one daemon-owned acquisition and
provider stack.

This is a pure-internal change: **external CLI behavior and output are
preserved** (the Python hooks in `integrations/` and the parallel
Claude channel adapter shell out to these verbs and parse their stdout).

## 2. Stages (sequenced, each independently reviewable)

0. **Build/green baseline.** The implementation started from a green unit +
   local-relay integration baseline. Current test tiers are documented in the
   README: CI uses `just test-unit`; local relay integrations use `just test`;
   public-relay probes are ignored and run only by explicit command.
1. `src/state.rs` uses `journal_mode=WAL`, `synchronous=NORMAL`, and
   `busy_timeout=5000`.
2. The UDS daemon owns startup locking, socket reclamation, the version
   handshake, and one `Store`; CLI verbs are thin RPC clients.
3. Per-session engines run as daemon-owned async tasks. The daemon's NMP host
   owns relay acquisition and group publication; `session_start` spawns an
   in-process `SessionTask`.

## 3. Process model

```
              ┌──────────────────────────── machine ────────────────────────────┐
              │                                                                   │
  hook /      │   ┌─────────────┐   UDS    ┌──────────────────────────────────┐  │
  CLI    ───▶ │   │ thin client │ ───────▶ │  mosaico daemon (single proc) │  │
 (one-shot)   │   │ (CLI verb)  │ ◀─────── │                                  │  │
              │   └─────────────┘  JSON    │  • owns state.db (one Store)     │  │   one
              │                            │  • owns NMP reads + writes ──────┼──┼──▶ relays
              │                            │  • doctor probe through NMP     │  │
              │                            │  • per-session async tasks       │  │
              │                            │  • chat / presence / pruning     │  │
              │                            │  • NIP-29 membership cache       │  │
              │                            └──────────────────────────────────┘  │
              └───────────────────────────────────────────────────────────────────┘
```

- The daemon is a normal `mosaico` invocation: `mosaico daemon`.
- The daemon runs the **tokio multi-thread runtime** (already how `main` builds
  it). It holds exactly one `Store` (single SQLite connection → one writer by
  construction) and one NMP engine for acquisition, account signing, and every
  runtime/profile write. Those writes enter NMP through the durable
  `submit_intents` queue, which owns routing, receipts, and retries. Bounded
  resolution reads and the doctor publish/readback use that same engine; the
  daemon has no second relay client.
- Per-session work runs as a tokio task inside the daemon (`SessionTask`), keyed
  by a private run key.

### Spawn-if-absent

Any invocation that needs state does `Daemon::connect_or_spawn()`:

1. Try to `connect()` to `$MOSAICO_HOME/daemon.sock` (default base
   `~/.mosaico`).
2. If that succeeds → return the client.
3. If it fails (no listener, or stale socket) → acquire the startup lock (§4),
   re-check (another racer may have just bound), and if still absent, **spawn**
   the daemon (double-fork / `setsid`, detach stdio → `~/.mosaico/daemon.log`),
   then poll-connect with a short timeout.

### Managed-session lifecycle

The daemon owns one durable lifecycle aggregate for each managed session. The
aggregate separates four facts that must not be inferred from one another:

- runtime incarnation and endpoint;
- presentation (`headed`, `headless`, or `unavailable`) plus attachment epoch;
- work (`working` or `idle`) and its generation-fenced eviction deadline;
- recovery authority and per-channel fabric standing.

PTY supervisors report attachment edges and child exit status. They also expose
an atomic conditional stop that succeeds only when the expected attachment
epoch is still headless, so an attach at the ten-minute boundary wins safely.
The daemon never treats a failed supervisor probe as headlessness. It persists
the presentation as `unavailable`, clears idle eligibility, and retains the
runtime for retry.

Every termination request for an admitted runtime enters one coordinator.
Automatic PTY termination is authorized only by the supervisor's atomic
zero-client check at the expected attachment epoch; an unavailable control
channel fails closed and has no raw-process fallback. Startup reconciliation
may inspect exact supervisor ownership to retain a runtime, but never uses that
ownership proof as permission to kill it. Explicit operator kill/forget may
terminate an attached runtime. Transport-specific PTY, ACP, and app-server
termination mechanics remain private executors behind that boundary, and the
durable stopped edge is committed only after process exit is confirmed.

When a headless, idle runtime has no pending delivery for ten minutes, the
lifecycle coordinator stops that exact incarnation. Its channel standing moves
to a persisted one-hour retention deadline. A clean successful child exit while
headed moves standing directly to absent. Both paths preserve exact recovery
identity and route affinity; only explicit forget or revoke destroys them.

Membership writes are serialized with lifecycle reconciliation. Expiry removes
standing only after the relay confirms it; a failed write remains retryable. An
authorized p-tag to a recoverable exact pubkey cancels stale eviction/removal
and re-admits absent standing. It resumes the native harness conversation when
a native locator exists; otherwise it fresh-launches the harness under the same
session pubkey. All timers and supervisor presentation are reconciled again
after daemon restart.

### Daemon process lifetime

The daemon remains running until an explicit stop, service-manager stop, or
protocol-skew re-exec. Runtime count does not control daemon lifetime: standing
expiry, exact p-tag recovery, relay retry work, and persisted supervisor-exit
reports all require an owner while no agent process is running.

Detached PTY supervisors survive daemon replacement and are re-adopted. ACP and
app-server children instead use daemon-owned stdio, which cannot be reattached
after process replacement. An orderly daemon shutdown therefore terminates and
confirms every owned RPC process group and marks those runtime generations
superseded before releasing the registry. Their native resume locators and
standing remain available for exact recovery; no wrapper or provider process is
left orphaned.

## 4. Socket, lock, stale-reclaim, version handshake

Files (all under `$MOSAICO_HOME`, default `~/.mosaico`):

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
`end`) exists because `tail` needs server-push. One-shot verbs simply emit a
single `ok` and no `item`/`end`.

### Why the streaming framing matters (the key design call)

Walking each verb's true I/O shape:

| verb               | shape                | mechanism                                            |
|--------------------|----------------------|------------------------------------------------------|
| `session_start`    | one-shot             | daemon spawns a `SessionTask`, returns its public `pubkey` |
| `session_end`      | one-shot             | daemon stops the `SessionTask`                       |
| `turn_start`       | one-shot (may be big)| daemon drains chat + builds context, returns text    |
| `turn_check`       | one-shot, read-only  | peek chat; no writes                                 |
| `turn_end`         | one-shot             | flip turn state                                      |
| `channel_send`       | one-shot             | daemon publishes kind:9 chat event on the relay      |
| `channel_read`        | one-shot             | daemon returns chat history for the session/project  |
| `who`              | one-shot             | snapshot rows                                        |
| `who --live`       | client-side poll     | client calls `who` each refresh; renders terminal    |
| `doctor`           | one-shot             | daemon does the relay round-trip, returns result     |
| `tail`             | **stream**           | daemon pushes decoded fabric events until disconnect |

`tail` forces the protocol beyond simple req/resp: the client cannot open its own relay
  observation anymore (the daemon owns NMP acquisition), so the
  daemon must stream decoded events to the client as they arrive, indefinitely,
  until the client disconnects (Ctrl-C). This is what makes the `item`*/`end`
  streaming shape mandatory rather than optional.

Client disconnect (EOF / broken pipe on the UDS) is the universal cancel signal:
it drops a `tail` subscription forwarder.

## 6. RPC surface

The full method catalog lives in [daemon-rpc-surface.md](daemon-rpc-surface.md). The daemon design here keeps the process, ownership, and lifecycle model.
## 7. Engine ownership inside the daemon

The `session_start` RPC makes the daemon spawn a tokio task running
`runtime::run_session`:

- It uses the daemon's **shared** `Store` through the ownership model in §8.
- It declares per-session and per-channel live-query demand through the daemon's
  shared NMP host. NMP owns relay planning, connection repair, deduplication,
  and observation lifetimes. Incoming canonical additions cross a bounded,
  backpressured single-consumer channel into the materializer; relay bursts can
  slow observation drains but cannot silently drop read-model updates. Events
  are demuxed once, daemon-side, and routed to the right session chat queue(s).
  Mentions route via the `compute_targets` / `route_mention` logic over all running
  sessions.
- Profile publication, presence-lease renewal, and `watch_pid` death detection
  run in the per-session task. Managed lifecycle edges directly reconcile the
  generation-owned presence projection; there is no periodic semantic-state
  poll. Reconciled presence effects enter one bounded, ordered background queue,
  so a stalled relay cannot delay lifecycle RPCs or hooks. Every runtime and
  kind:0 profile write is signed and accepted through the shared NMP host's
  durable `submit_intents` queue.
- Peer-staleness pruning is a single daemon-level periodic task.

`EngineParams` is reused largely as-is, minus `store_path` (the task gets the
shared store) and with the daemon provider/NMP host injected.

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

- The NMP engine owns live queries, relay acquisition, local account
  capabilities, and the durable `submit_intents` queue for all runtime and
  profile writes, including pinned-host indexer delivery. It also owns bounded
  group/profile projections and the explicit doctor probe. No other component
  opens a relay connection.

Because there is exactly one writer process, the
multi-writer corruption class is eliminated regardless of WAL (WAL stays as
defense-in-depth + a small perf win).

## 8a. Multi-agent relay ownership

The daemon hosts one-pubkey identities, each with at most one active runtime. NMP
observes their shared demand publicly because Mosaico-created groups are public
and closed: anyone may read them, while only admitted members may write. Each author
registers a signer plus exact-account AUTH policy. Per-write identity overrides
authenticate as the frozen author; account changes never retarget another session's
accepted write. Relay sessions belong to NMP, never to an agent runtime.

1. **Cross-pubkey delivery.** A connection authenticated (NIP-42) as agent A
   *does* receive events p-tagged to a different agent B. The relay does **not**
   scope ordinary public event delivery to the connection's authed identity.
   Mosaico pins one shared public read demand to configured hosts; NIP-42 protected reads remain an NMP capability, not the product acquisition path for public groups.
2. **Multi-key publish.** NMP freezes the draft author at acceptance, selects the
   exact registered capability named by the write override, validates the signed
   result, authenticates the relay session as that author when challenged, and
   routes it to the NIP-29 host. Paths that immediately seed an exact local
   read-model row use NMP's serialized sign-only operation first, then hand that
   same signed event back to NMP as the durable payload.

## 8b. Demux + routing for multiple local agents (correctness)

Today `handle_incoming` / `route_mention` assume a single `me`. Inside the
daemon, "me" becomes the **set** of hosted local agent pubkeys:

- `is_self` = `local_pubkeys.contains(event.pubkey)` — skip our own
  profile/presence/activity/status for **any** hosted key.
- A `Mention` routes by `m.to_pubkey`: find the hosted identity whose pubkey
  equals `to_pubkey`, then deliver to that pubkey's active runtime if present
  (never another agent's runtime). Pending delivery remains owned by the pubkey
  across runtime replacement.
- Profile/presence/status from peers (non-local pubkeys) update the directory as
  today.

Test (added): a chat mention to A must land only in A's inbox, never B's; if A's
runtime is replaced before delivery, the replacement claims the same pending row.

## 8c. Session reconciliation on daemon (re)start (correctness)

Protocol-skew re-exec, service restart, or a daemon crash can stop a daemon while session
rows are still `runtime_state='running'`, and the new daemon's in-process
`SessionTask`s don't exist yet. On startup the daemon first replays persisted
supervisor-exit reports, resumes any fenced `stopping` eviction, and then
reconciles each running row:

- `watch_pid` set and `pid_alive(watch_pid)` → respawn a `SessionTask` for it;
- else → atomically stop that generation and begin its one-hour standing retention.

Without this, `who` and routing membership would lie after every daemon restart.

## 8d. Clientful-but-sessionless connections

The streaming `tail` RPC and `who --live` hold a client connection open
without owning a session. They do not affect runtime lifecycle or standing;
the daemon already persists independently until an explicit stop or re-exec.

## 8e. Working directory in presence and awareness (implemented)

Agents may run in different working directories or git worktrees on the same
backend. Presence/status therefore carries `rel_cwd`, computed relative to the
workspace root, so peers can distinguish `worktree1` from `worktree2` without
publishing an absolute home-directory path. The materialized status row retains
that value for the human `who` view and the agent `my session` briefing.

- The wire tag is `["rel-cwd", <rel>]` and is omitted for an empty value.
- Workspace-root cwd is represented as `.`.
- If no workspace base resolves, publishing falls back to the cwd basename.
- `host` is the configured backend label, not a DNS hostname.
- `remote` is derived by comparing exact backend labels.

Human `who` renders terminal-oriented fabric text and supports `--live`. Agent
`my session` and turn hooks share one capture, cursor-aware assembly, canonical
document, and XML renderer. Cursor `0` selects full state; later cursors select
per-node deltas while preserving the same schema, values, nesting, and escaping.

## 9. Landmines preserved (must not regress)

- **rustls ring CryptoProvider** install in `main()` stays — the daemon is now
  the process that touches the relay, so the install must run on its path too
  (it already runs in `main` before dispatch; `daemon` goes through `main`).
- **Identical standard-Nostr wire output** — the codec seam
  (`fabric::nip29::wire`) remains the event-shape authority; group writes route
  through NMP.
- **One NMP relay plane** serves standing observations, bounded
  diagnostic/resolution projections, the doctor probe, and every runtime or
  profile write. Writes are durably accepted through `submit_intents`; no
  direct client or parallel relay pool may be reintroduced.
- **NIP-29 membership semantics** — group creation, owner admin backfill, and
  agent member admission remain provider-owned and relay-authoritative. Local
  allow/block files are not part of the active NIP-29 path.

## 10. Tests

- **Daemon spawn race**: N threads call `connect_or_spawn()` simultaneously;
  assert exactly one daemon binds and all clients connect.
- **Stale-socket reclaim**: create a `daemon.sock` file with no listener; assert
  `connect_or_spawn()` unlinks + reclaims and connects.
- **RPC round-trip**: start a daemon (test home), issue `who` / `channel_send` /
  `channel_read` RPCs, assert results.
- **Version-skew handshake**: client with `protocol = N+1` against a daemon
  pinned to `N` → daemon exits, client respawns and succeeds; assert the old
  daemon process is gone.
- **Concurrency / corruption repro**: ~16 concurrent clients issue writes via
  the RPC path; assert a single writer (one daemon pid) and `PRAGMA
  integrity_check = ok` afterward. Keep a direct-`Store` stress test as the
  original-corruption regression repro so the thing we're fixing stays asserted.
- Keep the unit and local-relay integration suites green. Public-relay probes
  stay explicit because they publish disposable data to the configured relay.

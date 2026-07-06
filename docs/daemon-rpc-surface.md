# tenex-edge daemon RPC surface

Companion to [daemon-design.md](daemon-design.md). This file owns the durable wire-method catalog for the per-machine daemon.

## 6. RPC surface (every method)

Coarse, lifecycle/intent-level — **not** fine-grained DB ops. The engine lives
inside the daemon, so there is no per-DB-op RPC chatter; the surface is
low-frequency lifecycle signals from hooks, CLI reads, and chat writes.

All params/results are JSON. `session` fields are full session ids. For
agent-facing commands the daemon resolves the caller from the explicit session
when present, then the PTY session, harness-native session id, watched harness
process, and finally the cwd+agent scan where that fallback is safe. **The
resolution stays daemon-side** so every client path observes the same identity
rules.

### `session_start`
Spawns an in-daemon `SessionTask` (publishes profile, presence, subscribes,
distills, routes mentions — today's `runtime::run_session`).
```jsonc
params: {"agent": "coder", "session_id": "te-…"|null, "cwd": "/path", "watch_pid": 12345|null}
result: {"session_id": "te-…"}   // session_id printed verbatim to stdout
```
The `session_id` is the raw canonical id — an internal correlation handle for
hooks, PTY session binding, resume, and DB rows. It is never rendered as a
user-facing identity; a concrete agent instance is identified by its agent-instance
label (`haiku`, `haiku1`, …) backed by that instance's selected pubkey.
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
result: {"now": u64, "fabric": "<tenex-edge>…</tenex-edge>"|null,
         "rows": [ {source, fresh, slug, project, status, host,
                    session_id, age_secs}, … ]}
```
Returns the unified fabric context when a single current channel resolves. The
legacy `WhoSnapshot` rows remain the fallback for `--all-projects` and other
views without one channel scope. Agent-scoped `who` advances that session's
fabric cursor after rendering.

### `turn_start`
```jsonc
params: {"session": "te-…", "transcript": "/path"|null, "json": bool, "cwd": "/path"}
result: {"context": "…"|null}    // the assembled injection text, or null
```
Daemon marks the turn, records the transcript path, claims pending directed
mentions from the inbox ledger, and returns the same unified fabric context that
`who` uses. A first turn (`seen_cursor=0`) renders the relevant channel snapshot;
later turns render only rows changed since the session cursor. The cursor
advances after rendering. Empty session id ⇒ no-op (`context: null`).

### `turn_check`
```jsonc
params: {"session": "te-…"|null, "json": bool, "cwd": "/path", "env_session": "…"|null}
result: {"context": "…"|null}
```
Claims pending directed mentions once and uses a compare-and-swap cursor advance
for rate-limited fabric deltas. Hooks that lose the CAS emit no duplicate delta;
direct mentions still surface even when the delta window is closed.

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
params: {"id": "event-id"|null, "channel": "…"|null, "since": u64|null,
         "limit": u64|null, "offset": u64, "tail": bool, "live": bool, ...}
stream: {"item": {event_id, from_pubkey, from_slug, project, body,
                  truncated, created_at, ...}}
```
Streams channel chat from the relay-event cache. Normal history reads truncate
bodies past the fabric render limit and include `truncated=true`; exact
`--id`/`id` reads fetch one event by id and return the full body without channel
inference.

### `chat_write`
```jsonc
params: {"message": "…", "channel": "…"|null, "long_message": bool, ...}
result: {"event_id": "hex", "project": "channel-h", "mentioned_pubkey": "hex"|null,
         "mentioned_session": "te-…"|null, "mentioned_label": "agent"|null}
```
Publishes a NIP-29 kind:9 chat message from the caller's selected
agent-instance key and returns only after checked relay acceptance. Messages
over the fabric render limit are rejected unless `long_message=true`. `channel`
is destination targeting only; caller identity is resolved independently from
the session anchors.

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

### `statusline`
```jsonc
params: {"session": "te-…"|null, "cwd": "/path", ...}
result: {"working": bool, "status": "…", "session_count": N, "member_count": N,
         "is_member": bool, "pending": N, "pending_chat": N}
```
Pure-read snapshot for the host statusline integration — no drain, no writes.

### `who` (self-identity)
When run inside an agent session (`session`/`agent`/`group` signal present), `who`
attaches a `self` block — the caller's agent-instance label (`haiku`/`haiku1`),
selected pubkey, current channel, membership, and status.

### `ping`
```jsonc
params: {}
result: {"pong": true}
```
Health-check / keep-alive.

### `pty_status`
Returns live portable PTY session state known to the daemon.

### `pty_send`
Sends keystrokes or text to a portable PTY session.

### `pty_spawn`
Spawns a new portable PTY session for an agent, optionally pre-loading a message.

### `pty_attach`
Returns the PTY target string needed to attach to a session.

### `pty_resume`
Reconstitutes a dead harness session in pty (re-opens the agent in its worktree).

### `pty_resumable`
Returns the list of sessions that can be resumed (have no live PTY session but retain a session row).

### Control / handshake (not user verbs)
- `hello` / `welcome` (§4)
- `please_exit` (version-skew re-exec, §4)

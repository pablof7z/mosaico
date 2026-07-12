# tenex-edge daemon RPC surface

Companion to [daemon-design.md](daemon-design.md). This file owns the durable wire-method catalog for the per-machine daemon.

## 6. RPC surface (every method)

Coarse, lifecycle/intent-level — **not** fine-grained DB ops. The engine lives
inside the daemon, so there is no per-DB-op RPC chatter; the surface is
low-frequency lifecycle signals from hooks, CLI reads, and channel sends.

All params/results are JSON. Hook-owned `session` fields are internal canonical
session ids. Agent-facing resume selectors are full npub/hex pubkeys (or a
session's current leased handle), never raw session ids. The daemon resolves the caller from the explicit session
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
user-facing identity; a session is addressed by its dashed public handle, such
as `@quill-codex`, backed by the session's own minted pubkey. The npub is its
permanent copy-paste resume value; the handle is a seven-day offline lease.
The provider opens the workspace root NIP-29 group, named by the workspace slug,
and adds the session agent as a relay member before the engine publishes presence.
The workspace and root channel are one entity with the public address `<workspace>`.
There is no local agent
allow/block file in the NIP-29 path.

### `session_end`
```jsonc
params: {"session": "te-…"}
result: {"ended": true|false}    // false ⇒ no such session
```
Metadata-only. Stops the `SessionTask` (which publishes idle presence/status
and marks the session dead) but does **not** touch the hosted PTY/child
process — a process left running after `session_end` keeps executing
unsupervised. stderr message (`session … ended` / `no such session: …`) is
produced client-side to match today's output. CLI: `tenex-edge my session
end --self` (or a positional session id/alias in place of `--self`).

### `session_kill`
```jsonc
params: {"session": "te-…"}
result: {"killed": true|false, "ended": true|false, "note": "pty=…"|"pid=…", "reason": "…"}
```
Process-kill, the counterpart to `session_end`. Stops the session's hosted
process (kills the owning PTY if one is tracked, else `SIGTERM`s the tracked
child pid), then internally calls `session_end` to mark the session's
metadata dead. `killed` reflects whether process termination itself
succeeded; `reason` is populated on failure (including "no local session
matched" when `session` doesn't resolve). Reachable from the TUI's kill-session
keybinding and from the CLI: `tenex-edge my session kill --self`, which
resolves the caller from the PTY/session environment and refuses a
positional target — an agent may only kill its own session. The CLI exits
non-zero when `killed` is `false`.


### `who`
```jsonc
params: {"workspace": "…"|null, "all_workspaces": bool, "cwd": "/path"}
result: {"now": u64, "fabric": "<tenex-edge>…</tenex-edge>"|null,
         "fabric_human": "…"|null,
         "rows": [ {source, fresh, slug, channel, status, host,
                    session_id, age_secs}, … ]}
```
An exact live agent caller receives XML with one global `<agents>` capability
inventory and a `<workspaces>` inventory. Every known workspace is listed;
normally only the caller's workspace is expanded, while `all_workspaces` expands
all joined workspace blocks. The workspace is its root channel, represented as
`<workspace channel="workspace" ... members="N">`; only real descendants use
`<channel>` rows. Channel contents recurse only through channels containing the
caller. Members are typed as `<agent>` or `<human>` and backend management keys
are excluded. Capability rows carry `workspace-availability` from
materialized kind:30555 advertisements. A bare operator caller receives
terminal-oriented `fabric_human` text. Agent-scoped `who` advances that
session's fabric cursor after rendering.

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
params: {"session": "te-…"|null, "json": bool, "cwd": "/path"}
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
params: {"channel": "…"|null}
stream: {"item": {"line": "<rendered fabric line>"}}   // repeated
        … until client disconnects (Ctrl-C)
```
Daemon registers a forwarder on its shared relay subscription, decodes each
event with the codec, renders with the existing `render()` and streams the line.
The client just prints each `item.line`.

### `channel_read` (streaming)
```jsonc
params: {"id": "event-id"|null, "channel": "…"|null, "since": u64|null,
         "limit": u64|null, "offset": u64, "tail": bool, "live": bool, ...}
stream: {"item": {event_id, from_pubkey, from_slug, channel, body,
                  truncated, created_at, ...}}
```
Streams channel chat from the relay-event cache. Normal history reads truncate
bodies past the fabric render limit and include `truncated=true`; exact
`--id`/`id` reads fetch one event by id and return the full body without channel
inference.

### `channel_send`
```jsonc
params: {"message": "…", "channel": "…"|null, "long_message": bool, ...}
result: {"event_id": "hex", "channel": "channel-h", "mentioned_pubkey": "hex"|null,
         "mentioned_session": "te-…"|null, "mentioned_label": "agent"|null}
```
Publishes a NIP-29 kind:9 chat message signed by the caller's own
per-session key and returns only after checked relay acceptance. Messages
over the fabric render limit are rejected unless `long_message=true`. `channel`
is destination targeting only; caller identity is resolved independently from
the session anchors.

### `channel_reply`
```jsonc
params: {"id": "event-id-or-prefix", "message": "…", "long_message": bool, ...}
result: {"event_id": "hex", "reply_to": "hex", "channel": "channel-h",
         "mentioned_pubkey": "hex", "mentioned_session": "te-…"|null}
```
Publishes a threaded NIP-10 reply to an existing channel message. The daemon
resolves `id` against the channel read model, targets the original author's
pubkey, and signs the reply with the caller's per-session key.

### `propose`
```jsonc
params: {"title": "…", "body": "…", "session": "te-…"|null, "cwd": "/path", ...}
result: {"event_id": "hex"}
```
Publishes a NIP-29 proposal (structured suggestion) to the caller's current
channel.

### `root_channels`
```jsonc
params: {}
result: {"channels": [ {slug, about}, … ]}
```
Returns all known workspace root channels from the daemon's cache.

### `channel_edit`
```jsonc
params: {"channel": "…", "about": "…"}
result: {"event_id": "hex", "channel": "channel-h", "about": "…", "confirmed": true}
```
Publishes an updated NIP-29 kind:39000 group metadata event for a channel.

### `channel_members`
```jsonc
params: {"channel": "…"}
result: {"members": [ {pubkey, slug, role}, … ]}
```
Returns the current membership list for the given NIP-29 group.

### `channel_add_member`
```jsonc
params: {"channel": "…", "pubkey": "hex"|null, "agent": "slug"|null}
result: {"ok": true}
```
Adds a pubkey or agent to a NIP-29 group.

### `channel_remove_member`
```jsonc
params: {"channel": "…", "pubkey": "hex"}
result: {"ok": true}
```
Removes a pubkey from a NIP-29 group.

### `channel_create`
```jsonc
params: {"name": "…", "about": "…", "parent": "…"|null,
         "parent_channel": "…"|null, "agents": [...], ...}
result: {"child_h": "…", "display_path": "…", "switched": bool,
         "orchestration_event_id": "hex"|""}
```
Creates a child channel under the caller's current channel, an explicit parent,
or the `<workspace>` root resolved from cwd.

### `channel_list`
```jsonc
params: {"channel": "…"}
result: {"channel": "…", "rooms": [ {child_h, name, about, depth}, … ]}
```
Lists the materialized child-channel tree under a channel.

### `channel_join` / `channel_leave` / `channel_switch` / `channel_archive`
```jsonc
params: {"channel": "…", "session": "te-…"|null, ...}
result: {"channel": "channel-h", ...}
```
Mutates the caller session's channel membership or archives a channel.

### `statusline`
```jsonc
params: {"session": "te-…"|null, "cwd": "/path", ...}
result: {"working": bool, "status": "…", "session_count": N, "member_count": N,
         "is_member": bool, "pending": N, "pending_chat": N}
```
Pure-read snapshot for the host statusline integration — no drain, no writes.

### `who` (self-identity)
When an exact live session anchor resolves, `who` emits the agent XML projection
with a structured `<self>` row. Loose `agent`/`group` hints are insufficient and
never bind an arbitrary sibling session.

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
```jsonc
params: {"session": "npub1…"}
result: {"pty_id": "…", "npub": "npub1…", "agent": "coder"}
```

### `pty_resumable`
Returns resumable rows with `pubkey`, `npub`, and an optional current `handle`.
Raw session ids are not exposed.

### Control / handshake (not user verbs)
- `hello` / `welcome` (§4)
- `please_exit` (version-skew re-exec, §4)

# mosaico daemon RPC surface

Companion to [daemon-design.md](daemon-design.md). This file owns the durable wire-method catalog for the per-machine daemon.

## 6. RPC surface (every method)

Coarse, lifecycle/intent-level — **not** fine-grained DB ops. The engine lives
inside the daemon, so there is no per-DB-op RPC chatter; the surface is
low-frequency lifecycle signals from hooks, CLI reads, and channel sends.

All params/results are JSON. Public selectors are full npub/hex pubkeys or a
session's current leased handle, never private runtime ids. Harness-owned IDs
are typed locators, not identity. The daemon resolves the caller from the explicit public identity
when present, then the PTY locator, harness-native locator, watched harness
process, and finally the cwd+agent scan where that fallback is safe. **The
resolution stays daemon-side** so every client path observes the same identity
rules.

## Session lifecycle RPCs

The exact `session_start`, `session_end`, `session_kill`, and `session_pty_wrap`
contracts live in [daemon RPC session lifecycle](daemon-rpc-session-lifecycle.md).

### `who`
```jsonc
params: {"workspace": "…"|null, "all_workspaces": bool, "cwd": "/path"|null,
         "human_color": bool, "expired": false}
result: {
  "root": "…", "now": u64,
  "rows": [{
    "source": "Local"|"Peer",
    "state": "working"|"idle"|"suspended"|"offline",
    "slug": "…", "channel": "…", "status": "…", "activity": "…",
    "dormant": bool, "host": "…", "age_secs": u64|null,
    "rel_cwd": "…", "remote": bool,
    "work_root": "…", "work_root_display": "…", "pubkey": "hex"
  }, …],
  "other_roots": [{"root": "…", "agent_count": N,
                    "agents": ["…", …], "about": "…"|null}, …],
  "spawnable": [{"host": "…", "slug": "…", "command": "…",
                  "byline": "…"|null}, …],
  "channel_parent": "…"|null, "root_display": "…"
}
```
The normal snapshot result above is the exact serde shape of `WhoSnapshot`; the
nested row, other-root, and spawnable fields are exhaustive. The RPC may add a
top-level `fabric_human` string for terminal rendering. `expired: true` selects the alternate
`{"expired": [{"agent_slug", "pubkey", "npub", "handle", "host", "channel",
"last_seen", "resumable"}, …]}` result. The live snapshot and `my_session`'s XML
tree project the same canonical `WhoAggregation` store read, so channel,
session-state, capability, and live-status rules cannot drift.

### `agent_inventory`
```jsonc
params: {"cwd": "/path"|null}
result: {"agents": [{"slug": "…", "agent_slug": "…", "harness": "…",
                     "use_criteria": "…", "available_since": N,
                     "source": {…}}, …],
         "failures": ["…", …]}
```
Daemon-owned projection of durable keystore agents and detected native/PATH
capabilities. CLI listing and launch selection consume this RPC and never scan
the keystore, harness configuration, or native profile directories themselves.

### `agent_save`
```jsonc
params: {"slug": "…", "harness": "…",
         "profile": "…"|null, "per_session_key": bool|null}
result: {"created": bool, "slug": "…", "harness": "…"}
```
Strict daemon-owned create/update of one durable agent configuration. `slug` and
`harness` are required; `profile` and `per_session_key` may be omitted (the same
as `null`). Unknown fields or wrong JSON types are rejected. Slugs accept only
`[A-Za-z0-9._-]`; harness/profile names are trimmed and must be non-empty when
present. A null/omitted profile clears the stored profile. A null/omitted
`per_session_key` preserves an existing identity mode and defaults a new agent
to per-session identity. `created` distinguishes create from update; the result
returns the persisted slug and normalized harness.

### `agent_remove`
```jsonc
params: {"slug": "…"}
result: {"removed": bool}
```
Strict daemon-owned permanent removal. `slug` is the only accepted field and
uses the same validation as `agent_save`; missing, unknown, or wrongly typed
fields are rejected. `removed` is false only when no configured agent file
exists for that slug.

### `my_session`
```jsonc
params: {"pty_session": "…"|null, "harness_session": "…"|null,
         "watch_pid": N|null, ...}
result: {"fabric": "<mosaico>…</mosaico>"}
```
Strict self-scoped agent briefing. It resolves the exact live caller and emits
`<self>`, global `<agents>` capabilities, all known workspaces, nested channels,
and typed member sessions. Every workspace joined by this exact session is
expanded; merely known workspaces stay compact. This is a pure read and does
not advance the hook-awareness cursor.

### `my_session_status`
```jsonc
params: {"title": "…", exact caller anchor fields...}
result: {"title": "…"}
```
Sets and immediately publishes the exact caller session's broadcast
status/title. CLI: `mosaico my session status <TITLE>`.

### `turn_start`
```jsonc
params: {"harness_session": "native-id", "transcript": "/path"|null, "json": bool, "cwd": "/path"}
result: {"context": "…"|null}    // the assembled injection text, or null
```
Daemon marks the turn, records the transcript path, claims pending directed
mentions from the inbox ledger, and returns the hook fabric context. A first
turn (`seen_cursor=0`) renders the relevant channel snapshot;
later turns render only rows changed since the session cursor. The cursor
advances after rendering. An absent harness locator yields `context: null`.
Hook context does not embed the agent roster, and roster-only updates do not
emit a delta.

### `turn_check`
```jsonc
params: {"harness_session": "native-id"|null, "json": bool, "cwd": "/path"}
result: {"context": "…"|null}
```
Claims pending directed mentions once and uses a compare-and-swap cursor advance
for rate-limited fabric deltas. Hooks that lose the CAS emit no duplicate delta;
direct mentions still surface even when the delta window is closed.

### `turn_end`
```jsonc
params: {"harness_session": "native-id"}
result: {"ok": true}
```

### `doctor`
```jsonc
params: {}
result: {"relays": [...], "probe_pubkey": "hex", "publish": "OK …"|"ERR …",
         "readback": "N event(s) …"|"ERR …"}
```
The daemon's narrow direct edge performs the connectivity publish + read-back;
the client prints the existing multi-line report. Product group writes do not
use this diagnostic path.

### `tail` (streaming)
```jsonc
params: {"channel": "…"|null}
stream: {"item": {"line": "<rendered fabric line>"}}   // repeated
        … until client disconnects (Ctrl-C)
```
The daemon ensures NMP observation coverage for the requested channel, then
forwards structured events emitted by the materializer and daemon lifecycle.
Backfill comes from the canonical store; live events come from the daemon's
bounded tail broadcast. The client renders each streamed item.

### Channel messaging
The streaming read, send, reply, and blocking wait contracts live in
[daemon-rpc-messaging.md](daemon-rpc-messaging.md).

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
params: {"channel": "…", "session": "npub1…"|"hex"|"handle"|null, ...}
result: {"channel": "channel-h", ...}
```
Mutates the caller session's channel membership or archives a channel.

### `statusline`
```jsonc
params: {"harness_session": "native-id"|null, "cwd": "/path", ...}
result: {"working": bool, "status": "…", "session_count": N, "member_count": N,
         "is_member": bool, "pending": N, "pending_chat": N}
```
Pure-read snapshot for the host statusline integration — no drain, no writes.

### `ping`
```jsonc
params: {}
result: {"pong": true}
```
Health-check / keep-alive.

### `pty_status`
Returns live portable PTY state with `pty_id`, `pubkey`, `npub`, and optional
public `handle`; private runtime ids are omitted.

### `operator_sessions`
Returns the canonical local control projection consumed by `mosaico
sessions`. It starts from `runtime_state='running'` rows in the daemon-owned `sessions` table,
but exposes only `pubkey`, `npub`, and the current public `handle`; the private
runtime row id never crosses this RPC boundary. Each row joins agent/harness
state, workspace-grouped joined channels, filesystem bindings, local host, and
an optional typed endpoint `{id, kind, live, attachable, cwd, command}` whose
liveness and attachability are projected by its owning transport. Remote
relay-only status rows are intentionally
excluded; they remain observable through `who` and cannot be killed by this
machine.

### `pty_send`
Sends keystrokes or text to a portable PTY session.

### `pty_spawn`
Spawns an agent through either its explicit bundle binding or an unambiguous
logical native/generic provider. This interactive boundary selects PTY launch
policy and atomically creates the canonical zero-argument bundle when none is
configured, optionally pre-loading a message. The RPC accepts no argv, command,
or bundle override.

### `pty_attach`
Accepts an npub, hex pubkey, or handle and returns the PTY target plus public identity.

### `pty_resume`
Reconstitutes a stopped harness session in PTY (re-opens the agent in its worktree).
```jsonc
params: {"session": "npub1…"}
result: {"pty_id": "…", "npub": "npub1…", "agent": "coder"}
```

### `pty_resumable`
Returns resumable rows with `pubkey`, `npub`, `runtime_state`, and an optional current `handle`.
Raw session ids are not exposed.

### Control / handshake (not user verbs)
- `hello` / `welcome` (§4)
- `please_exit` (version-skew re-exec, §4)

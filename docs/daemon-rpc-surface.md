# mosaico daemon RPC surface

Companion to [daemon-design.md](daemon-design.md). This file owns the durable wire-method catalog for the per-machine daemon.

## 6. RPC surface (every method)

Coarse, lifecycle/intent-level — **not** fine-grained DB ops. The daemon-owned
engine exposes only low-frequency lifecycle signals from hooks, CLI reads, and
channel sends.

All params/results are JSON. Public selectors are full npub/hex pubkeys or a
session's leased handle, never private runtime ids. Harness IDs are typed
locators, not identity. Caller resolution prefers explicit public identity, then
PTY or harness-native locators, watched process, and finally safe cwd+agent
scanning. It stays daemon-side so every client observes identical rules.

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
The result is the exhaustive serde shape of `WhoSnapshot` and may add a top-level
`fabric_human` terminal rendering. `expired: true` instead selects
`{"expired": [{"agent_slug", "pubkey", "npub", "handle", "host", "channel",
"last_seen", "resumable"}, …]}`. Live and `my_session` XML views share the same
canonical `WhoAggregation` read so their state and capability rules cannot drift.

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
Strict daemon-owned create/update. `slug` and `harness` are required; optional
`profile` and `per_session_key` treat omission as null. Unknown or wrongly typed
fields are rejected. Slugs accept `[A-Za-z0-9._-]`; harness/profile names are
trimmed and non-empty. Null profile clears it; null `per_session_key` preserves
existing identity mode and defaults new agents to per-session. `created`
distinguishes creation; the result returns persisted slug and normalized harness.

### `agent_remove`
```jsonc
params: {"slug": "…"}
result: {"removed": bool}
```
Strict daemon-owned permanent removal. `slug` is the only accepted field and
uses the same validation as `agent_save`; missing, unknown, or wrongly typed
fields are rejected. `removed` is false only when no configured agent file
exists for that slug.

### `backend_profile_refresh`
```jsonc
params: {}
result: {"scheduled": true}
```
Refreshes daemon-owned agent discovery and schedules publication of one complete
management-key kind:0 host profile containing all host agents (with compact
`about`) and known workspace roots. Replacement is atomic; exact-author
observations keep management/admin profiles current without a global feed.

### `my_session`
```jsonc
params: {"pty_session": "…"|null, "harness_session": "…"|null,
         "watch_pid": N|null, ...}
result: {"fabric": "<mosaico>…</mosaico>"}
```
Strict self-scoped agent briefing. It resolves the exact live caller, requests
canonical state at cursor `0`, and emits `<self>`, host agents, workspaces,
channels, and member sessions. A workspace lists only advertising hosts whose
management keys are current admins; rows expose neither repeated channel ids nor
local paths. Each workspace contains its root `<channel>` and member data belongs
only to channels. IDs are absolute slash paths (`/<workspace>/<child>`). Channels
on the exact session's active route omit `members`; inactive channels retain the
count; expanded channels may expose typed member rows. Non-member channels with accepted chat expose relative `last-active`. This pure read does not advance the hook-awareness cursor.

### `my_session_status`
```jsonc
params: {"title": "…", exact caller anchor fields...}
result: {"title": "…"}
```
Sets and immediately publishes the exact caller session's broadcast
status/title. CLI: `mosaico my session status <TITLE>`.

### `turn_start`
```jsonc
params: {"harness_session": "native-id", "json": bool, "cwd": "/path"}
result: {"context": "…"|null}    // the assembled injection text, or null
```
Daemon marks the turn and claims pending directed
mentions from the inbox ledger, and returns the hook fabric context. A first
turn (`seen_cursor=0`) renders the relevant channel snapshot;
later turns render only rows changed since the session cursor. The cursor
advances after rendering. An absent harness locator yields `context: null`.
`my_session` and hooks use the same capture, assembly, and XML renderer. Profile
capabilities follow their own cursor delta policy like other canonical nodes.

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
bounded tail broadcast. The client renders each streamed item. `channel` here is
a raw opaque `channel_h`, not a channel reference.

### Channels
The channel addressing contract (every `"channel"` argument is a full absolute
path `/workspace/child` or an `@<id-prefix>`, resolved globally and exactly)
and the channel lifecycle/membership RPCs — `root_channels`, `channel_edit`,
`channel_members`, `channel_add_member`, `channel_remove_member`,
`channel_create`, `channel_list`, `channel_join`, `channel_leave`,
`channel_archive` — live in [daemon-rpc-channels.md](daemon-rpc-channels.md).

### Channel messaging
The streaming read, send, reply, and blocking wait contracts live in
[daemon-rpc-messaging.md](daemon-rpc-messaging.md).

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
machine. A local managed-RPC row may additionally contain `native_outcome`
with `{outcome, delivery_kind, delivery_event_id, native_thread_id,
native_turn_id, error_message, error_details, finished_at}`. This diagnostic is
separate from `state`; a failed native turn never invents a fifth presence
state.

### `pty_send`
Sends keystrokes or text to a portable PTY session.

### `pty_spawn`
Spawns an agent through either its explicit bundle binding or an unambiguous
logical native/generic provider. This interactive boundary selects PTY launch
policy and atomically creates the canonical zero-argument bundle when none is
configured, optionally pre-loading a message. The RPC accepts no argv, command,
or bundle override. Its response confirms the session is registered and ready;
it does not claim that an optional opening prompt completed.

### `pty_attach`
Accepts an npub, hex pubkey, or handle and returns the PTY target plus public identity.

### `pty_resume`
Reconstitutes a stopped harness session in PTY (re-opens the agent in its worktree).
```jsonc
params: {"session": "npub1…"}
result: {"pty_id": "…", "npub": "npub1…", "agent": "coder"}
```

### `pty_resume_native`
Resolves a harness-native id across typed local locators, then attaches or
resumes the exact mapped Mosaico session. When no mapping exists, authoritative
local harness storage may adopt it under that harness's generic agent identity.
Mapped rows preserve their persisted agent slug and signer even when current
agent-profile configuration is stale or absent. Running non-PTY sessions are
never double-spawned. Full contract: [daemon RPC session
lifecycle](daemon-rpc-session-lifecycle.md#pty_resume_native).

### `pty_resumable`
Returns resumable rows with `pubkey`, `npub`, `runtime_state`, and an optional current `handle`.
Raw session ids are not exposed.

### Control / handshake (not user verbs)
- `hello` / `welcome` (§4)
- `please_exit` (version-skew re-exec, §4)

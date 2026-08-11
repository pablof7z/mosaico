# mosaico daemon RPC surface

Companion to [daemon-design.md](daemon-design.md). This file owns the durable wire-method catalog for a selected-instance daemon.

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

### `agent_key_status`
```jsonc
params: {"slug": "…"}
result: {"status": "absent"|"ready"|"missing"}
```
Strict daemon-owned launch preflight for persisted agent identity. `missing`
means the configured agent has `perSessionKey: false` but lacks either its
secret or public key. Malformed keys and mismatched complete pairs are errors,
not repair candidates.

### `agent_key_create`
```jsonc
params: {"slug": "…"}
result: {"created": bool}
```
Atomically completes missing durable key material after the interactive client
has obtained confirmation. A valid existing secret is preserved and its public
key is derived; otherwise one fresh matching pair is persisted. The RPC rejects
per-session agents and never returns secret material.

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
canonical state at cursor `0`, and emits `<self>`, host inventory, a root-channel
forest, and typed member sessions. There are no workspace wrappers and no
repeated channel name/id attributes. Public names are absolute hash paths
(`#root/child`); opaque protocol ids and local paths are never exposed. Every
root containing one of the session's joined channels expands recursively.
Other roots remain compact. Member rows appear only where the session belongs;
non-member channels may expose an agent-only count and relative `last-active`.
This pure read does not advance the hook-awareness cursor.

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
result: {
  "relays": [...],
  "probe_pubkey": "hex"|null,
  "write_probe": {
    "publish": {
      "status": "verified"|"skipped"|"failed",
      "summary": "…",
      "terminal": "Settled"|"…",
      "relays": [{"relay": "wss://…", "state": "published"|"rejected"|"auth_failed"|"gave_up"|"waiting"|"sent", "reason": "…"|null}]
    },
    "readback": {
      "status": "verified"|"failed",
      "summary": "…",
      "acquisition": {"termination": "relay_settled"|"coverage_proven"|"timed_out"|"subscription_closed", "branches": [...]}
    }
  },
  "publish_queue": {...}
}
```
The daemon's narrow direct edge waits up to five seconds for NMP's typed terminal
signal, then asks NMP for the reduced terminal receipt and preserves every relay
result. A write that does not finish inside the health bound stays in NMP's
durable queue; doctor fails rather than hanging or calling custody a relay ACK.
Publish verifies only when every configured destination reports `Published`.
Readback verifies only after every planned source reaches end-of-stored-events;
cached rows and coverage-proven cache remain visible but do not impersonate
current relay I/O. With no authorized group, a relay-settled metadata query is
the read-only connectivity proof. Product writes do not use this diagnostic path.

Doctor writes share one replaceable `kind:30078` / `d=mosaico-doctor` coordinate
per signing identity. Later runs supersede older unsent probes instead of
accumulating obligations; the per-run `t` marker still proves current readback.

### `tail` (streaming)
```jsonc
params: {"channel": "#root/child"|null}
stream: {"item": {"category": "…", "channel": "#root/child", …}} // repeated
        … until client disconnects (Ctrl-C)
```
The daemon resolves the requested full path, ensures NMP observation coverage,
then forwards structured events emitted by the materializer and daemon
lifecycle. Backfill comes from the canonical store; live events come from the
daemon's bounded tail broadcast. Every streamed `channel` is a full public path;
opaque protocol identifiers remain inside the daemon.

### Channels
The channel addressing contract (every public `"channel"` argument is a full
absolute path `#workspace/child`, resolved globally and exactly)
and the channel lifecycle/membership RPCs — `root_channels`, `channel_edit`,
`channel_members`, `channel_add_member`, `channel_remove_member`,
`channel_create`, `channel_list`, `channel_join`, `channel_leave`,
`channel_archive`, `channel_delete` — live in
[daemon-rpc-channels.md](daemon-rpc-channels.md).

### Channel messaging
The streaming read, local-cache search, send, reply, and blocking wait
contracts live in
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

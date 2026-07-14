# `tenex-edge tail` v2 — design spec

> Design produced by an opus design agent before the canonical message read model
> landed. Current code has `messages` and `message_recipients`; it does not have a
> standalone `threads` table. Use `messages.thread_id` where this spec mentions
> thread rows.

## 1. Purpose
`tail` is the operator's **live activity feed for the whole fabric** — a
scrolling, structured log answering: *"What are my agents doing right now, and
what just changed?"* Distinct from `who` (point-in-time roster snapshot) and
`inbox` (one agent's received messages). `tail` is fabric-wide, time-ordered
change across all agents + message traffic. Read-only; never mutates inbox/seen.

## 2. What it shows (categories)
- **msg** (never suppressed): directed message/mention, with thread short-id,
  sender→recipient (+ session short-code when targeted), 72-col body snippet.
- **sync**: outbound delivery lifecycle `pending→accepted→delivered|failed`.
  Show only terminal/notable (accepted/delivered/failed); suppress `pending`.
- **turn**: working/idle transitions only (the working↔idle edge), with elapsed
  duration on idle.
- **stat**: NIP-38 status text changes; suppress unchanged; debounce 2s per
  (agent,project); idle status folds into the turn idle line.
- **join/leave**: presence ONLINE/OFFLINE transitions only — raw periodic
  heartbeats NEVER emit a line (first heartbeat = join, expiry/end = leave).
- **sess**: local own-session start/end.
- **proj**: project metadata (about) changed; dedupe identical text.
- **id/profile**: first-discovery only; default-hidden.

### Suppress/collapse (the noise fixes)
Heartbeats → join/leave transitions; identical status → deduped; profile →
first-discovery, hidden; sync `published` → suppressed; activity → debounced.

## 3. Noise control
1. Heartbeat→transition conversion (removes ~90% of volume).
2. Per-source debounce/dedupe keyed by (category, agent, project, thread?).
3. Severity tiers: **signal** (msg, turn, sync failed, join, leave),
   **ambient** (status, sync delivered/accepted, sess, proj), **noise**
   (profile, raw beats).
4. Default-hidden categories (profile, heartbeats); `--include`/`--all` re-enable.

## 4. Format
Line grammar: `<TS>  <cat>  <agent@project[sess]>  <verb/glyph> <detail>`
- TS: wall-clock `HH:MM:SS` default; `--relative` for `12s ago`.
- cat: fixed 5-char colored tag (msg=yellow, sync=cyan/red, turn=green,
  stat=magenta, join=green, leave=dim, sess=blue, proj=dim).
- identity: `slug@project[sess]` (peers: `slug@host[sess]`); the agent is the
  agent-instance label (`haiku`, `haiku1`, …) backed by its selected pubkey, and
  `sess` is a short prefix of the raw canonical `session_id` — an operator
  correlation handle only, never a user-facing identity.
- glyphs: `▶` started, `⏸` idle, `→` message, `✗`/failed.
  ASCII fallback via `--no-emoji` (`>`,`||`,`->`,`[x]`).
- thread id `#xxxx` (4-char short of thread/root id); same thread = same code.
- body: straight quotes, truncate 72 cols + `…`, newlines→spaces; `-v` raises.
- color only on TTY; respect NO_COLOR.

### Sample (the real 3-agent threaded review)
```
14:30:38  sess   claude@tenex-edge[a3f1]  session start (rel_cwd: .)
14:30:40  join   codex@tower[7c20]        online (tenex-edge, .)
14:31:50  turn   claude@tenex-edge[a3f1]  ▶ started working
14:32:01  msg    claude@tenex-edge[a3f1]  → codex[7c20]  #b8e2 "can you review the codec seam refactor?"
14:32:02  sync   claude → codex           #b8e2 delivered
14:32:48  msg    codex@tower[7c20]        → claude  #b8e2 "looks good — one nit: doc the seam test"
14:33:10  msg    opencode@tower[e91a]     → claude  #b8e2 "+1, fold idle-status into turn end"
14:33:21  turn   claude@tenex-edge[a3f1]  ⏸ idle (1m31s)
14:48:12  leave  opencode@tower[e91a]     offline (was online 17m)
```

## 5. Modes & flags
Default (bare `tail`): follow live, all projects/hosts, wall-clock, color on TTY,
tiers action+signal+ambient (hide profile/heartbeats), backfill last ~20 events.

Flags: `--workspace`, `--agent`, `--host`, `--since <dur|ts>`, `--backfill N`
(default 20; 0 = pure live), `--only <cats>`, `--exclude <cats>`,
`--include profile`, `--all`/`-v`, `--compact`/`-q`, `--no-follow` (history dump
+ exit), `--relative`, `--no-emoji`/`--no-color`, `--json` (NDJSON of raw
TailEvents), `--live` (TUI dashboard — follow-up).

`--live` dashboard (follow-up): full-screen TUI grouped by project→agent, each
agent row = slug@host, online dot, working/idle + live elapsed, current status,
last-message-at; bottom pane = last ~10 msg/sync/turn events. Same event stream.

## 6. Implementation notes
### A. Pivot stream from pre-rendered strings to structured events
Replace `{ "line": "<ansi>" }` with a tagged `TailEvent` JSON so the CLI owns
formatting/filtering:
```rust
#[derive(Serialize)]
#[serde(tag = "category", rename_all = "snake_case")]
enum TailEvent {
    Msg{ts,project,from,to,thread,body},
    Sync{ts,project,from,to,thread,state,detail},
    Turn{ts,project,agent,session,state,elapsed_s},
    Status{ts,project,agent,text},
    Join{ts,project,agent,host,session,rel_cwd},
    Leave{ts,project,agent,host,session,online_s},
    Sess{ts,project,agent,session,state,rel_cwd},
    Proj{ts,project,about},
    Profile{ts,agent,host,pubkey},
}
```
Add an internal `emit_tail(state, TailEvent)` fanning out on a
`broadcast::Sender<TailEvent>`; RPC handlers push transition events the raw
`DomainEvent` bus can't represent. `handle_tail` sends
`Response::item(id, to_value(tail_event))`. CLI `tail()` deserializes + renders
locally (`render_tail_event(&TailEvent,&Opts)`); `--json` prints verbatim.

### B. Emit at mutation sites (don't diff the firehose)
- RPC handlers: `rpc_turn_start/end`→Turn (elapsed from turn_state),
  `rpc_session_start/end`→Sess, `rpc_send_message`→Msg + Sync (use the
  SendIntent thread_id).
- `handle_incoming` / materializer: decoded Mention→Msg (inbound),
  39000→Proj on change, new Profile→Profile.

### C. Derive join/leave
join = first time a peer session_id is seen (first_seen); subsequent beats emit
nothing. leave = prune/expiry sweep (prune_peer_sessions / Presence.expires_at)
emits Leave{online_s}. Local leave = rpc_session_end. Keep a small in-memory
HashMap<session_id,(first_seen,project,slug,host)> on DaemonState.

### D. Backfill
`{backfill:N, since:ts}` param on handle_tail. Backfill from the canonical
`messages`/`message_recipients` tables (recent messages as Msg w/
`messages.thread_id`) + a roster snapshot from list_alive_sessions/list_peer_sessions/
get_turn_state/agent_status as synthetic Join/turn/stat, sorted by ts, before
the live loop. (The original spec's inbox-only fallback is unnecessary here.)

### E. Thread short-ids
Derive `#xxxx` from message thread_id (or root native id), via the existing
hash short-code helper, so a conversation visually groups.

### F. Scope
Quick wins: structured TailEvent + emit_tail; emit at the RPC handlers +
handle_incoming branches; heartbeat→join/leave; CLI render/filter/tiers/flags;
read-model backfill; `--json`. Follow-ups: `--live` TUI; persisted append-only
event log for deep `--since`.

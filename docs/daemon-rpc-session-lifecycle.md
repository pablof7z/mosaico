# Mosaico daemon session lifecycle RPCs

Companion to [daemon RPC surface](daemon-rpc-surface.md). This file owns the
durable wire contracts for session admission, termination, and PTY re-homing.

## `session_start`

Spawns an in-daemon `SessionTask` (publishes profile and presence, declares its
NMP live-query demand, and routes mentions — today's `runtime::run_session`).

```jsonc
params: {"agent": "coder", "observed_harness": "claude-code", "claimed_harness": "claude-code"|null, "admitted_bundle": "claude-pty"|null, "admitted_transport": "pty"|"acp"|"app-server"|null, "endpoint_provenance": "launch"|"hook", "pty_session": "endpoint-id"|null, "endpoint_kind": "pty"|"acp"|"app-server"|null, "profile": "reviewer"|null, "harness_session": "native-id"|null, "cwd": "/path", "watch_pid": 12345|null}
result: {"pubkey": "hex"}
```

`harness_session` is a typed harness locator and never identity. A session is
addressed by its dashed public handle, such as `@quill-codex`, backed by the
session's own minted pubkey. The npub is its permanent copy-paste resume value;
the handle is a seven-day offline lease. The provider opens the workspace root
NIP-29 group through NMP, names it from the workspace slug, and adds the session
agent as a relay member before the engine publishes presence.

Launch admission persists `observed_harness`, bundle, transport, and endpoint
provenance as immutable facts for that runtime. A hook reports its host string
separately as `claimed_harness`; the adapter derives `observed_harness` from the
owned launch environment or a recognized ancestor process. Missing or unknown
observations fail instead of being guessed from locator shape. Claims are kept
for mismatch diagnostics and never reclassify a launch-owned runtime, including
when a dead row is reasserted. Endpoint access always uses the session's exact
`(observed_harness, locator_kind)` address. An endpoint is accepted only with an
explicit matching `endpoint_kind`; missing or mismatched kinds fail before
locator resolution or persistence.

The workspace and root channel are one entity with the public address
`<workspace>`. There is no local agent allow/block file in the NIP-29 path.

## `session_end`

```jsonc
params: {"session": "npub1…"|"hex"|"handle"}
result: {"ended": true|false}    // false ⇒ no such session
```

Metadata-only. Stops the `SessionTask` (which publishes idle presence/status
and marks the session dead) but does **not** touch the hosted PTY/child process
— a process left running after `session_end` keeps executing unsupervised.
stderr message (`session … ended` / `no such session: …`) is produced
client-side to match today's output. CLI: `mosaico my session end --self`;
agents cannot target another session.

## `session_kill`

```jsonc
params: {"session": "npub1…"|"hex"|"handle", "revoke_memberships": bool}
result: {"killed": true|false, "ended": true|false, "note": "endpoint=…"|"pid=…", "cleanup_confirmed": bool, "cleanup_failures": ["…"], "reason": "…"}
```

Process-kill, the counterpart to `session_end`. Stops the session's hosted
endpoint through its transport if one is tracked, else `SIGTERM`s the tracked
child pid, then internally calls `session_end` to mark the session's metadata
dead. `killed` reflects whether process termination itself succeeded; `reason`
is populated on failure (including "no local session matched" when `session`
doesn't resolve). `mosaico sessions` sets `revoke_memberships`: the daemon also
expires presence now, clears the resume claim, confirms removal from every
recorded NIP-29 channel, and clears local channel bindings. `mosaico my session
kill --self` leaves that flag false, resolves the caller from the PTY/session
environment, and refuses a positional target — an agent may only kill its own
session. The CLI exits non-zero when process termination or requested fabric
cleanup is not confirmed.

## `session_pty_wrap`

```jsonc
params: {"session": "npub1…"|"hex"|"handle"}
result: {"wrapped": true, "pty_id": "…"}
       | {"wrapped": false, "refusal": "already_wrapped"|"working"|"not_resumable"|"not_found"|"kill_failed"|"resume_failed", "reason": "…"}
```

Re-homes a session started manually outside a daemon-owned PTY (no live
`pty_session` alias, so idle mentions silently black-hole — see
`turn_context::start`'s warning) into a fresh daemon PTY supervisor. Refuses if
the session already has a live `pty_session` alias (`already_wrapped`, nothing
to do), is mid-turn (`working`, to avoid losing in-flight work), or carries no
harness resume token (`not_resumable`). Otherwise kills the manually-started
process (via `session_kill`, marking the old row dead) before resuming the same
harness session inside a fresh PTY, so the two steps cannot race a second caller
across CLI round-trips. Only the harness's own persisted session state survives
the hop; terminal scrollback from the killed process is lost. CLI: `mosaico my
session pty-wrap-me --self`, which resolves the caller from the PTY/session
environment and refuses a positional target — an agent may only re-home its own
session. The CLI exits non-zero unless the refusal is `already_wrapped`.

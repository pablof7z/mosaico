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
params: {"session": "npub1…"|"hex"|"handle"|null,
         "harness_session": "native-id"|null, "harness": "…"|null,
         "cause": "manual"|"harness_hook"}
result: {"ended": true|false, "deferred": true|absent}
```

Stops exactly the resolved runtime generation but does **not** signal its
hosted process. A process left running after a manual `session_end` therefore
keeps executing unsupervised. A PTY harness hook is deferred because only the
supervisor can atomically classify child status with attachment state: a clean
zero-status exit while headed is intentional user exit, while a headless or
non-clean exit retains standing for one hour.
Manual calls resolve the explicit public `session`; hooks resolve the same
daemon-owned caller anchor chain as other lifecycle RPCs, including the typed
native harness locator when no public identity is present.
stderr message (`session … ended` / `no such session: …`) is produced
client-side to match today's output. CLI: `mosaico my session end --self`;
agents cannot target another session.

## `session_kill`

```jsonc
params: {"session": "npub1…"|"hex"|"handle", "forget": bool}
result: {"killed": true|false, "ended": true|false,
         "recovery_revoked": true|absent,
         "note": "endpoint=…"|"pid=…", "cleanup_confirmed": bool,
         "cleanup_failures": ["…"], "reason": "…"}
```

Process-kill, the counterpart to `session_end`. Stops the session's hosted
endpoint through its transport if one is tracked, else `SIGTERM`s the tracked
child pid, then marks the exact generation stopped. `killed` reflects whether
process termination itself was confirmed; `reason`
is populated on failure (including "no local session matched" when `session`
doesn't resolve).

`forget: true` is the destructive recovery boundary used by the operator
session picker. The daemon first persists `recovery_state='revoked'`, then
re-reads and terminates the current generation. If termination fails, the
runtime and locators remain tracked for retry but exact recovery stays revoked.
Only after confirmed process absence does one transaction stop the runtime and
remove signer, route, and locator authority; relay removals remain durable
retry work until confirmed. `mosaico my session kill --self` leaves `forget`
false and may only kill the caller's own session.

Ordinary stops do not revoke recovery. A stopped pubkey retains channel
standing for one hour, and exact p-tag routing can re-admit it after standing
expires. A native resume locator restores the same provider conversation;
without one, Mosaico launches a fresh provider conversation under the same
session pubkey.

## `pty_resume_native`

```jsonc
params: {"native_id": "harness-owned-id", "workspace": "/absolute/path"|null}
result: {"action": "attached"|"resumed"|"adopted", "pty_id": "…",
         "pubkey": "hex", "npub": "npub1…", "handle": "quill-codex",
         "agent": "developer", "harness": "claude-code"}
```

Operator entry point for `mosaico resume <HARNESS_ID>`. The daemon first looks
up the native locator across every harness. A mapped locator resumes the exact
persisted pubkey, signer, agent slug, workspace, and channel; current agent
profile configuration contributes no identity authority. A live PTY attaches,
while a running non-PTY runtime refuses to double-spawn and directs explicit
takeover to `mosaico sessions`.

An unmapped id is adopted only when authoritative local Claude, Codex, Grok, or
OpenCode storage identifies one harness. Its recorded cwd selects the workspace
unless `workspace` supplies an existing absolute directory. Mosaico then mints
the generic per-session identity for that harness and atomically claims the
native locator before opening the PTY. Missing and cross-harness-ambiguous ids
fail; UUID shape is never a harness signal.

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

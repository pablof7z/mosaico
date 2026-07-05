# Trellis adoption

tenex-edge derives live resources — relay subscriptions, kind:30315 status,
and the injected hook/fabric-context snapshot — from changing state. Those
derivations and the effects that follow are now owned by
[Trellis](https://github.com/pablof7z/trellis), a deterministic reconciliation
engine: state changes go in; effect plans and receipts come out.

## Boundary principle

**Trellis owns decisions; the host owns observations and effects.**

```
observed facts (the world hands us)
  → a Trellis transaction decides new semantic state
  → Trellis emits resource commands / output frames (plain data)
  → tenex-edge applies them (sign, publish, subscribe, render)
  → success/failure re-enters as new observed facts
```

Facts Trellis must **not** invent — they enter as canonical inputs: a hook
happened, a process is alive/dead, a transcript window was captured, the LLM
returned `NOW: doing x`, the relay accepted/rejected an event, a clock tick.
The input-journal vocabulary is `src/reconcile/journal.rs` (`InputFact`); each
variant names the writer it replaces.

Facts Trellis **does** own after that: a session is working, the activity
should be *doing x*, publish this exact 30315, this subscription should close,
this hook snapshot has this shape — and the causal path that explains each one.

Bulky payloads (full transcripts, raw event bodies) never enter the graph — only
stable hashes/summaries/keys. Trellis is a control plane, not a blob store.

## No split-brain

For each surface, exactly one authority decides and **every** writer routes
through it. There is no path that mutates a surface's effect directly beside the
reconciler. The old parallel mutators were collapsed in the same change that
introduced each reconciler (e.g. the second status heartbeat timer and the
direct `set_status` seam were deleted, not left alongside).

## The surfaces (`src/reconcile/`)

| Surface | Module | Model | What it fixed |
|---|---|---|---|
| Subscriptions | `subscriptions/` | per-entity `ResourceKey` refcounted by per-session scopes | the unbounded-subscription leak — channel-leave now emits a real NIP-01 CLOSE on last-owner departure |
| Status (kind:30315) | `status/` | per-session derived `StatusContent` → publish/expire commands | five triggers + two timers collapsed to one change-only publish path; dedup; deterministic expiry on death; h-tag correction on leave |
| Hook context | `hook_context/` | derived `FabricView` → materialized output frame | the hand-rolled `turn_start_audit` that drifted from the render, replaced by a receipt that *is* the render's dependency trace; cursor + `now` made explicit inputs (deterministic/replayable) |

## Retrospective instrumentation (`tenex-edge explain`)

Every distill round-trip records an `llm_calls` row (the exact transcript slice,
system prompt, model, raw response, keyed by a sha256 `window_hash`). Every
reconciler commit records a `receipts` row (surface, transaction, changed
summary, commands, `artifact_ref` = the published event id). The same
`window_hash` threads distill → status publish → receipt.

Replay capsules live in `trellis_replay_capsules` as versioned
`DataTransactionScript<InputFact>` JSON captured at the drive seam. Retention is
bounded to the newest 512 capsules and 16 MiB of serialized script bytes; the
same off-values used by `TENEX_EDGE_HOOK_CALL_LOG` also disable capsule capture
unless `TENEX_EDGE_REPLAY_CAPSULES` overrides the gate.

`tenex-edge probe simulate <surface> --fact '<InputFact JSON>'` stages one fact
against the daemon-held status or subscription graph and calls
`Transaction::preview()` instead of committing. The returned plan is the resource
commands and changed labels that would result; the live revision stays unchanged.
For those authoritative surfaces, the live effect seam also previews the same
fact/snapshot before applying host effects and blocks the effect if the committed
plan does not match the previewed plan.

```
tenex-edge explain event:<30315-id>   # the receipt + the exact LLM inputs behind the activity
tenex-edge explain hook:<session>[@ts] # why the injected snapshot had this shape
tenex-edge explain llm:<id> | session:<id>[@ts] | txn:<surface>:<id> | sub:<channel>
```

`--json` for the raw joined record; `--redact` to replace prompt/transcript
bodies with `sha256:<hash> (<n> bytes)`.

## Self-check (the oracle) and CI

Every reconciler test calls `assert_incremental_equals_full()` after each
transaction — Trellis's oracle rebuilds all derived state from canonical inputs
and compares it to the incrementally-maintained state. The hook-context surface
additionally ships `determinism_and_replay` (same inputs → identical snapshot
*and* identical receipt) and `equivalence_with_legacy_build_view` (byte-for-byte
against the pre-Trellis renderer). All of these run under `cargo test --lib`
(`just test-unit`) — the CI contract — so incremental/full divergence or a
render regression fails the build.

## Adoption boundary left imperative

`rpc_session_start` remains effect-imperative by design: it performs DB writes,
relay checks, signer admission, tmux stamping, subscriptions, replay, and engine
spawn. It is nevertheless advisory now: `InputFact::SessionStartRequested`
derives the staged row/check/admit/subscription/spawn intents, the RPC executes
that plan, and `SessionStarted`/`SessionStartFailed` outcome facts feed the graph
back. Each request logs a one-request shadow comparison (`shadow_matches=1`,
`shadow_total=1` when the derived plan matches the host-observed intent) and
records replay capsules for the request/outcome facts. This surface is capped at
advisory because Trellis can prove its graph bookkeeping, not the external
effects themselves.

Cursor advancement is graph-owned: render requests enter as
`InputFact::TurnCheckRequested`, the daemon-held cursor graph derives `HookFrame`
or `NoFrame`, and the host only applies the resulting `sessions.seen_cursor`
projection. The non-status direct publishers remain explicit follow-ups.

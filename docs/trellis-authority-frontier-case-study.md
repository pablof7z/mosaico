# tenex-edge: moving the authority frontier under a live agent fabric

Date: 2026-07-05

This is the closeout artifact for issue #240 and the case-study layer of the
Authority Frontier epic (#228). The core question was whether Trellis merely
observes tenex-edge state, or now leads the state transitions that are safe for
it to lead.

## Result

Trellis is authoritative for six of the seven registered surfaces:

| Surface | Start of #228 | Current mode | Evidence |
|---|---:|---:|---|
| status | authoritative, under-instrumented | authoritative | previewed status plan, replay capsules, oracle, stats |
| subscriptions | authoritative, under-instrumented | authoritative | refcounted resources, open/close balance, replay corpus |
| hook_context | advisory | authoritative | daemon-held graph, render receipts, state/why probes |
| turn_lifecycle | imperative | authoritative | staged facts, projection executor, bypass ratchet |
| cursor | imperative CAS | authoritative | staged `TurnCheckRequested`, projection executor, bypass ratchet |
| outbox | imperative | authoritative | enqueue/result facts, queue projection executor, bypass ratchet |
| session_start | imperative | advisory | staged intents + request/outcome facts; capped by design |

`probe seams` reports 85% host-seam coverage: six authoritative surfaces out of
seven registered surfaces. `session_start` intentionally stops at advisory
because Trellis can derive the intended row/check/admit/subscription/spawn plan,
but cannot prove SQLite, relay, signer, tmux, and process-spawn effects.

## Exp A: diagnosis metric

Corpus: `tests/fixtures/trellis_diagnosis/{leaked-close,false-republish}.json`
with the scoring rubric in `ground-truth.md`.

Diagnostic inputs:

1. **Trace + labels**: serialized replay script, surface, step names, resource
   command counts, and stable labels such as
   `subscriptions/session/s1/channels` and `status/s1/activity`.
2. **Raw-log surrogate**: the same step names and coarse command counts with
   labels, resource handles, and causal vocabulary stripped. No stored raw
   `tracing` fixtures exist for this seeded corpus, so this is a conservative
   baseline definition rather than a literal daemon log capture.

Scoring: an answer was correct only if it named the root cause in the ground
truth, not merely the affected surface.

| Case | Trace + labels diagnosis | Score | Raw-log surrogate diagnosis | Score |
|---|---|---:|---|---:|
| leaked-close | owner/refcount collapse: first departing owner must not close a resource still owned by `s2` | 1 | session leave caused a close; shared-owner cause not identifiable | 0 |
| false-republish | status dedup disabled: unchanged same-bucket tick must not publish again | 1 | duplicate publish after tick suggests dedup failure | 1 |

Measured result on the two-case corpus:

- Trace + labels: 2/2 = 100%.
- Raw-log surrogate: 1/2 = 50%.
- Delta: +50 percentage points.

Honesty note: this is a smoke metric, not a statistical claim. The corpus has
only two seeded bugs, and this closeout was not blind because the repo already
contains the ground truth. The useful result is that the structural Trellis
receipts make the shared-owner subscription bug diagnosable, where coarse logs
do not.

## Exp B: safety invariant

Invariant: on authoritative surfaces, no host effect executes without a
previewed Trellis plan first.

Evidence in code:

- `probe simulate` calls `Transaction::preview()` and returns plans without
  mutating live graph revisions.
- `src/reconcile/frontier/tests.rs::authoritative_effect_executors_require_preview_evidence`
  ratchets the effect executors for status, subscriptions, turn lifecycle,
  cursor, and outbox.
- `src/daemon/server/probe/simulate/tests.rs` verifies preview-only behavior for
  status, subscriptions, cursor, outbox, and new status-session labels.
- `src/daemon/server/probe/tests.rs::rpc_probe_reflects_driven_state_for_every_verb`
  exercises the real probe RPC over a daemon state and checks `simulate`
  preserves the revision.

Current validation evidence:

```text
cargo fmt -- --check
cargo check --all-targets
cargo test --lib --quiet
cargo clippy --all-targets -- -D warnings
bash scripts/check_loc.sh
bash scripts/check_integration_helpers.sh
git diff --check
just test-local-relay
OWNER_PUBLIC_KEY=79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798 NIP29_RELAY_BIN=/tmp/croissant-smallmap/croissant just test-local-nip29
```

The invariant is scoped to authoritative surfaces. `session_start` is advisory,
so it records and compares staged Trellis intent but is not counted as a proven
authoritative effect surface.

## Beachhead: subscription leak fix

The original beachhead was #205: a shared channel subscription must close only
after the last owner leaves. The focused live-probe proof is
`src/daemon/server/probe/tests.rs::rpc_probe_stats_quantifies_shared_subscription_beachhead`.
It registers two live sessions in `room`, syncs subscriptions through the daemon
executor, marks the first owner dead, calls `probe stats`, then marks the second
owner dead and calls `probe stats` again.

`probe stats --surface subscriptions` after the first owner leaves:

```json
{
  "open_count": 2,
  "close_count": 0,
  "latest_graph_resources": 2,
  "resource_drift": false
}
```

Final `probe stats --surface subscriptions` after the last owner leaves:

```json
{
  "open_count": 2,
  "close_count": 2,
  "live_resource_balance": 0,
  "latest_graph_resources": 0,
  "resource_drift": false
}
```

The `N` is 2 because one channel owns two narrow REQs: chat/status `#h` and
group-state `#d`. The live balance reaches zero and `resource_drift=false`, so
the measured result is: 2 closes, 0 leaked live resources.

## Upstream Trellis asks

All seven asks from #228 are now filed or explicitly linked:

| Ask | Trellis link | Status |
|---|---|---|
| `Transaction::preview()` dry run | pablof7z/trellis#163 | filed, closed |
| Serialize audit explanations into trace exports | pablof7z/trellis#165 | filed |
| Historical `why_at(revision)` over retained audit data | pablof7z/trellis#166 | filed |
| First-class node-label registry export | pablof7z/trellis#167 | filed |
| Projection-frame pattern and single-output guidance | pablof7z/trellis#168, related #121 | filed |
| #114 status stale relative to #141 | pablof7z/trellis#114, PR #141 | already filed/closed |
| Host-conformance ledger pattern for `trellis-testing` | pablof7z/trellis#169 | filed |

## Bottom line

Trellis is now guiding the safe state-derivation surfaces, not merely auditing
them after the fact. The boundary is deliberately honest: status,
subscriptions, hook context, turn lifecycle, cursor, and outbox are
authoritative; session start is advisory because its external effects cannot be
made graph-proven without lying about what Trellis controls.

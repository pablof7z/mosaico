---
type: research-record
date: 2026-06-28
session: b9176726-a9a8-41a9-b806-c966e8c94ed7
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b9176726-a9a8-41a9-b806-c966e8c94ed7.jsonl
source_lines: 2904-2927
agent_attribution: main
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Ordinal allocation investigation via e2e harness: reproduced same agent (smith) with ordinal-0 alive in both ord-demo and ord-room2 simultaneously, root cause identified in per-room slot reservation key"
captured_at: 2026-06-28T15:33:03Z
---

Ordinal allocation investigation via e2e harness: reproduced same agent (smith) with ordinal-0 alive in both ord-demo and ord-room2 simultaneously, root cause identified in per-room slot reservation key

---

```
ALIVE 'smith' sessions:           ALIVE identities (signing key):
sid              pubkey    chan    signpk      ord  chan
te-…737326d      6b6dc747  ord-demo   db4008a5  1   ord-demo
te-…fb8f5060     6b6dc747  ord-demo   6b6dc747  0   ord-room2   ← base key, LIVE
te-…fb91136a     6b6dc747  ord-room2
```

The base key `6b6dc747` (ordinal 0) is **alive in both `ord-demo` and `ord-room2` at once** — so it heartbeats kind:30315 to both channels as the same author. That's exactly your `4077e280` → `test1` + `test1-c028b76a` + `omnigent`.

## Root cause — proven, one line
`session_signer.rs:6`: *"At most one LIVE session per `(base agent pubkey, room h, ordinal)`."* The reservation slot is keyed `slot(base_pubkey, **h**, ordinal)` — **the room is in the key.** So:
- Same room, second session → ordinal 0 taken → bump to ordinal 1 (distinct key). ✅ works (see `db4008a5` ord-1 in `ord-demo`).
- **Different** room → that room has never seen the agent → hands out **ordinal 0 = the base key again.** ❌ Now the base identity is live in N rooms simultaneously.

There is no global "this identity is already live somewhere" check, and no cross-room supersession. The per-room `h` in the slot key directly contradicts the model's own stated invariant ("at most one live session per ordinal").

## The fix (consistent with the existing ordinal model)
**Drop `h` from the live-reservation key** — make it `(base_pubkey, ordinal)` global across all rooms. Then a second concurrent session of the same agent, *in any room*, finds ordinal 0 taken → gets ordinal 1 → a **distinct** pubkey. One key is never live in two channels; "where is pubkey X" has exactly one answer. A session that *moves* channels (`channels switch`) keeps its ordinal and just updates `channel_h` — that's already in place. This is your "single source of truth," and it keeps concurrency (the whole point of ordinals).

(The alternative — model A, supersede the old session on any channel move so an agent is only ever in one channel — is a bigger behavior change and throws away concurrency. I don't recommend it; the ordinal machinery already exists to do this right.)

This also explains the harness's **check-4 FAIL** (`channels switch` didn't reject a live-ordinal collision) — same missing global-liveness check.


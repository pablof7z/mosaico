---
type: research-record
date: 2026-06-26
session: b429fe81-7956-4a43-a87f-94e1799bf6e3
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b429fe81-7956-4a43-a87f-94e1799bf6e3.jsonl
source_lines: 4925-4964
agent_attribution: main
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Validation report of NIP-29 admin invariant implementation across three test scenarios: top-level channel creation, subgroup admin inheritance, multi-agent join"
captured_at: 2026-06-26T10:07:48Z
---

Validation report of NIP-29 admin invariant implementation across three test scenarios: top-level channel creation, subgroup admin inheritance, multi-agent join

---

## Validation Report: NIP-29 Admin Invariant (H1–H4 Fixes)

**Test environment:** local croissant relay at `ws://127.0.0.1:19988`, fresh data directory (`/tmp/croissant-fresh-v/`). Config: `TENEX_CONFIG=/tmp/te-test-7FcZ9O/config.json` pointing entirely to the local relay (both `relays` and `indexerRelay` set to `ws://127.0.0.1:19988`).

### Scenario 1 — Top-level project channel: PASS

Started a daemon and session against a fresh relay. The channel `te-proj-alpha` was provisioned from scratch.

**Expected:** mgmt key + whitelisted key as admins; developer agent as member.  
**Observed (via `nak req` on 39001/39002):**
- 39001: both `09417c93` (mgmt) and `09d48a1a` (whitelisted) listed as admins ✓  
- 39002: all three (mgmt + whitelisted + developer agent) listed as members ✓

### Scenario 2 — Subgroup inherits parent admins: PASS

Created `wasm-child` as a NIP-29 subgroup under `te-proj-alpha`.

**Expected:** child inherits parent's admins (mgmt + whitelisted) on top of any child-specific admins.  
**Observed:**
- 39001 for `wasm-child`: same mgmt + whitelisted admins carried over from parent ✓  
- 39002 for `wasm-child`: mgmt + whitelisted + developer agent as members ✓

The `propagate_parent_admins_to_children` path was exercised here.

### Scenario 3 — Second agent joins existing channel: PASS

Added a second agent (`architect`, pubkey `bb50e2d8…`) to the already-provisioned `te-proj-alpha`.

**Expected:** new agent added as member without H1 cache collision (the stale `(channel)` → `(channel, expect_member)` key fix).  
**Observed:**
- architect correctly appeared in 39002 for `te-proj-alpha` ✓  
- No stale "already ready" shortcut blocked the re-check ✓

### Side-finding: daemon subscription flooding

The daemon opens several concurrent subscriptions per channel (39000/39001/39002 metadata, 30315 status, kind:9 moderation, kind:0 profiles). With even 2–3 active channels, khatru's `maxPendingMsgs = 1000` goroutine limit kills the WebSocket connection, the daemon reconnects, and the loop restarts. **This doesn't affect correctness** (the relay state was correct in all three scenarios) but it is a latent stability issue for multi-channel deployments. Patched the threshold to 50000 locally for testing; the real fix would be reducing concurrent subscription cardinality in the daemon.

### Production spot-check

Independently queried `wss://nip29.f7z.io` directly — all previously-provisioned groups (`te-proj-alpha`, `tenex-edge`, `wasm2`, session rooms) had correct admin invariant already in place from prior daemon runs with the H1-H4 fixes applied.

---
type: research-record
date: 2026-06-28
session: b9176726-a9a8-41a9-b806-c966e8c94ed7
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b9176726-a9a8-41a9-b806-c966e8c94ed7.jsonl
source_lines: 1673-1688
agent_attribution: main
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Independent verification of schema rewrite: cargo build succeeds, cargo test --lib passes 255/0 tests, integration crates identified with 77 compile errors"
captured_at: 2026-06-28T12:45:54Z
---

Independent verification of schema rewrite: cargo build succeeds, cargo test --lib passes 255/0 tests, integration crates identified with 77 compile errors

---

Assistant: Verified status — and one real gap the workflow under-reported:

## ✅ Core rewrite landed and independently verified
- **Schema is exactly the 11 agreed tables** (`relay_channels/_channel_members/_profiles/_status/_events` + `sessions/session_aliases/identities/inbox/outbox/project_roots`).
- **`cargo build` (lib+bins) clean** — 5 dead-code warnings only.
- **`cargo test --lib` = 255 passed / 0 failed**, including new invariant tests that actually prove the design: NIP-01 replacement (addressable *and* replaceable), pubkey-unique-per-channel (admin supersedes member), idempotent register-per-external-id, outbox retry.
- **−5,785 LOC net** (3,048 added, 8,833 deleted). `owns_group`, `is_session_room`, the three peer tables, the dead Phase-1 model, `processed_orchestration` — all gone.

## ⚠️ Gap: integration test crates don't compile
The workflow only ran `cargo test --lib`. The **`tests/` integration crates have 77 compile errors** — all mechanical staleness against the old API (`Session.project`→`channel_h`, `is_group_member`→`is_channel_member`, `upsert_project_meta`→`upsert_channel`, plus refs to removed `is_session_room`/`is_group_owned`/`route_scope`). No structural problems, just an unmigrated test harness.

## ⚠️ Still true (unchanged)
- Stale *comments* in a few files still name old tables (`presence_state`, `processed_orchestration`) — cosmetic.
- **No live e2e through the hook path** — your standing rule. A clean build ≠ a working daemon.

I'm launching a focused agent to migrate the integration test crates to the new API and iterate to green — this is bounded, mechanical work.

---
type: research-record
date: 2026-06-28
session: b9176726-a9a8-41a9-b806-c966e8c94ed7
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b9176726-a9a8-41a9-b806-c966e8c94ed7.jsonl
source_lines: 2202-2222
agent_attribution: main
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Integration test failures: deterministic bug vs. timing race investigation — verdict: TIMING FLAKINESS (harness fragility under concurrent load), not logic regression"
captured_at: 2026-06-28T15:14:58Z
---

Integration test failures: deterministic bug vs. timing race investigation — verdict: TIMING FLAKINESS (harness fragility under concurrent load), not logic regression

---

Assistant: Now I have the answer with evidence. A **different** test failed on the second loaded run (`signers::nak_relay_observes_transient_duplicate_status_author`), and the suite passed **30/30** on the very next run. So this isn't one broken test — it's load-induced flakiness spread across the heavy integration tests.

## Diagnosis: timing flakiness, not a logic regression

| Evidence | Reading |
|---|---|
| `chat_write_stdin...` passes **6/6 in isolation** | not a deterministic bug |
| Full suite: run 1 fails `signers::...`, run 2 passes **30/30** | failures are non-deterministic and move between tests |
| Both failing tests poll a real daemon + `nak` relay round-trip with bounded timeouts | the window occasionally loses under full-suite concurrency (~100s, multiple daemons/relays) |
| The delivery invariant itself holds — the inbox poll confirms the row lands; `drain_pending_for_session` is non-destructive | routing/materialization logic is sound |

These are real-daemon/real-relay integration tests with timing windows; under concurrent load a publish→relay→subscribe→materialize round-trip sometimes exceeds the test's poll budget. That's harness fragility, not a schema-logic fault. The committed `fbb50ee6` is sound for what unit + isolated integration tests can prove.

## What I'd do about it
The honest fix is **test hardening, not timeout-padding to hide a bug**: give the single-shot assertions (e.g. the statusline check) the same bounded retry-poll the inbox check already uses, since async delivery is inherently eventually-consistent — production hooks already tolerate "not yet, next hook catches it." I won't touch the tests for that without your go, because bumping waits can also mask a genuine slowdown.

Two honest unknowns I can't close from here:
- Whether the rewrite made per-event materialization *marginally* slower (widening the window) vs. pure machine contention — would need timing instrumentation under load to separate.
- The standing **e2e-through-the-hook-path** gate is still unmet.

Want me to (a) harden the flaky assertions with retry-polls, (b) instrument materialization timing to rule out a real slowdown, or (c) leave it and move to the room→channel terminology / e2e?

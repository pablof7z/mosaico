---
type: research-record
date: 2026-06-28
session: b9176726-a9a8-41a9-b806-c966e8c94ed7
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b9176726-a9a8-41a9-b806-c966e8c94ed7.jsonl
source_lines: 2138-2213
agent_attribution: main
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Integration test timing flakiness under concurrent load vs logic regression: six isolated runs pass, full suite runs flake non-deterministically, verdict is harness fragility not logic fault"
captured_at: 2026-06-28T15:33:03Z
---

Integration test timing flakiness under concurrent load vs logic regression: six isolated runs pass, full suite runs flake non-deterministically, verdict is harness fragility not logic fault

---

Assistant: No uncommitted changes, and the test is **flaky** — passed, failed, passed across three identical runs. The agent's "30/30" caught a lucky run; this is a real timing race, not a fixed test. Let me read the test and the chat-routing path to determine whether it's a test-side wait issue or a genuine race in the rewrite.

User: [Tool result]: 180	        &[("TENEX_EDGE_SESSION", "chat-sender-session")],
181	    );
182	    assert!(
183	        out.status.success(),
184	        "chat read failed\nstdout: {}\nstderr: {}",
185	        String::from_utf8_lossy(&out.stdout),
186	        String::from_utf8_lossy(&out.stderr)
187	    );
188	    let stdout = String::from_utf8_lossy(&out.stdout);
189	    // The author slug/host now resolves from the relay-cached `relay_profiles`
190	    // (materialized from kind:0). A local agent whose kind:0 isn't materialized in
191	    // this nak-relay env renders the short-pubkey fallback, so assert the body and
192	    // timestamp render (the sender identity is checked deterministically below via
193	    // the inbox `from_pubkey`).
194	    assert!(
195	        stdout.contains(&format!("> {body} [")),
196	        "chat read should render the body and a timestamp; got: {stdout}"
197	    );
198	
199	    // The sender's authoring pubkey (its ordinal-0 durable agent key) is what the
200	    // inbo

Assistant: The smell is at line 211: the migration renamed `peek_chat` → **`drain_pending_for_session`**. `peek` is non-destructive; `drain` consumes. The poll loop drains the row, then the statusline assertion at 235 expects it still pending. Let me confirm `drain` is destructive and what `statusline`'s `pending` reads from.

User: [Tool result]: chpwd:5: command not found: _iterm2_set_profile
=== drain_pending_for_session: destructive? ===
    pub fn drain_pending_for_session(&self, target_session: &str) -> Result<Vec<InboxRow>> {
        // Fall back to the raw id when no canonical mapping exists — symmetric
        // with `enqueue_inbox`, which parks under the same raw id in that case.
        let target = self
            .resolve_canonical_id(target_session)?
            .unwrap_or_else(|| target_session.to_string());
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             WHERE target_session=?1 AND state='pending' ORDER BY created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
=== what does statusline 'pending' read? ===
src/daemon/server/statusline.rs:19:/// NEVER write to state.db (no drains, no touches) — peeks only.
src/daemon/server/statusline.rs:77:        let pen

Assistant: `drain_pending_for_session` is actually **non-destructive** (a plain `SELECT ... WHERE state='pending'` — the name is a misnomer), and statusline reads the same way. So it's not a consume race. The earlier panic was at `messaging.rs:232` — the `.expect("pending array")` / `pending.iter()` region. Rather than theorize, let me capture the real failure with daemon logs.

User: [Tool result]: chpwd:5: command not found: _iterm2_set_profile
run 1: ok
run 2: ok
run 3: ok
run 4: ok
run 5: ok
run 6: ok

Assistant: Passes 6/6 in isolation — so it's not a deterministic logic bug; it only failed under full-suite concurrency. That points to a load-sensitive timing flake. Let me try to catch it under the actual condition (full suite) to be definitive rather than guess.

User: [Tool result]: Exit code 1
chpwd:5: command not found: _iterm2_set_profile
### suite run 1: test result: FAILED. 29 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 101.47s
test signers::nak_relay_observes_transient_duplicate_status_author ... FAILED
thread 'signers::nak_relay_observes_transient_duplicate_status_author' (34157425) panicked at tests/daemon_integration/signers.rs:200:5:
### suite run 2: test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 83.68s

Assistant: Now I have the answer with evidence. A **different** test failed on the second loaded run (`signers::nak_relay_observes_transient_duplicate_status_author`), and the suite passed **30/30** on the very next run. So this isn't one broken test — it's load-induced flakiness spread across the heavy integration tests.

## Diagnosis: timing flakiness, not a logic regression

| Evidence | Reading |
|---|---|
| `chat_write_stdin...` passes **6/6 in isolation** | not a deterministic bug |
| Full suite: run 1 fails `signers::...`, run 2 passes **30/30** | failures are non-deterministic and move between tests |
| Both failing tests poll a real daemon + `nak` relay round-trip with bounded timeouts | the window occasionally loses under full-suite concurrency (~100s, multiple daemons/relays) |
| The delivery invariant itself holds — the inbox poll confirms the row lands; `drain_pending_for_session` is non-destructive | routing/materialization logic is sound |

These are real-daemon/real-relay integration tests with timing windows; under concurrent load a publish→relay→subscribe→materialize round-trip sometimes exceeds the test's poll budget. That's harness fragility, not a schema-logic fault. The committed `fbb50ee6` is sound for what unit + isolated integration tests can prove.

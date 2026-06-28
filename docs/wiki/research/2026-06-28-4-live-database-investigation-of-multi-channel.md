---
type: research-record
date: 2026-06-28
session: b9176726-a9a8-41a9-b806-c966e8c94ed7
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b9176726-a9a8-41a9-b806-c966e8c94ed7.jsonl
source_lines: 2497-2549
agent_attribution: main
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Live database investigation of multi-channel bug: schema check and SQL queries on new-schema DB show 3 session rows for same agent across different channels, falsifying prior impossibility claim"
captured_at: 2026-06-28T12:45:54Z
---

Live database investigation of multi-channel bug: schema check and SQL queries on new-schema DB show 3 session rows for same agent across different channels, falsifying prior impossibility claim

---

Assistant: Two separate problems here, and the first thing I must establish is whether `/tmp/te` is running the **new** binary — because if it is, my "structurally impossible" claim is falsified by this very log (same pubkey → `test1`, `test1-c028b76a`, *and* `omnigent`). Let me check the binary/schema at that home and find the source of the empty `#d`/`#h` filters.

User: [Tool result]: chpwd:5: command not found: _iterm2_set_profile
=== /tmp/te schema: new or old? ===
## /tmp/te/state.db
relay_status
relay_channels
  has channel_h col in sessions:
1

=== do these REQ sub-names / kind 39003 exist in NEW code? ===
src/state/schema.rs:27:CREATE INDEX IF NOT EXISTS idx_relay_channel_members_pubkey
src/daemon/server.rs:210:    log_nip29_role_decision, refresh_project_members_cache, rpc_debug_outbox, rpc_doctor,
src/daemon/server/diagnostics.rs:35:pub(in crate::daemon::server) async fn refresh_project_members_cache(
src/daemon/server/diagnostics.rs:84:        let refreshed = refresh_project_members_cache(state, project).await;
src/daemon/server/channels_rpc.rs:319:    refresh_project_members_cache(state, &new_channel).await;
src/daemon/server/rpc/project.rs:85:    refresh_project_members_cache(state, &p.project).await;
src/fabric/nip29/materializer.rs:248:    fn admins_and_members_preserve_each_other() {
src/fabric/nip29/lifecycle.rs:98:/// relay29 advertises the `admin`/`

Assistant: `/tmp/te/state.db` is **new-schema** (`relay_status`, `relay_channels`, `channel_h`) — so the new binary ran there. But the REQ log you pasted uses sub-names (`mux_chat_`, `meta_`, `roles_`/39003) that **don't exist in the new code** — those are the *old* binary. So the two logs are likely from different daemons. The decisive test of my "impossible" claim is the live new DB itself. Checking it now:

User: [Tool result]: chpwd:5: command not found: _iterm2_set_profile
=== ALL session rows (the multi-channel question) ===
sid|ap|agent_slug|channel_h|alive|last_seen
te-18bd3ee|4077e28010|developer|omnigent|1|1782650505
te-18bd3ea|4077e28010|developer|test1|0|1782650198
te-18bd3e9|4077e28010|developer|test1-c028b76a|1|1782650503

=== rows per (agent_pubkey): >1 alive in different channels == the bug ===
ap|rows|channels|group_concat(DISTINCT channel_h)
4077e28010|2|2|omnigent,test1-c028b76a

=== aliases for that agent (how many harness sessions?) ===
external_id_kind|ext|sid
harness_session|4054e0c2-6fdb-4c|te-18bd3e9
tmux_pane|%58|te-18bd3e9
watch_pid|21103|te-18bd3e9
harness_session|134de40a-5847-4b|te-18bd3ea
tmux_pane|%59|te-18bd3ea
watch_pid|42172|te-18bd3ea
harness_session|d1e5cdd5-6b0f-44|te-18bd3ee
watch_pid|50236|te-18bd3ee

Assistant: I was wrong — and the live new-schema data proves it. Let me be precise about what's actually happening:

**Three session rows for the same agent `4077e28010` (developer), three distinct harness ids:**
- `te-18bd3e9` → `test1-c028b76a`, **alive**, harness `4054e0c2`, pid 21103
- `te-18bd3ea` → `test1`, dead, harness `134de40a`, pid 42172
- `te-18bd3ee` → `omnigent`, **alive**, harness `d1e5cdd5`, pid 50236

So my "structurally impossible" claim was too narrow. Each *row* has one `channel_h` — but the **same agent fragments into N session rows across N channels**, because `register_session` keys identity on the harness-native id, and the harness mints a *fresh* id per spawn/channel-move. Two alive rows → two heartbeat channels. That's the real bug, and it's live in the new code. Let me confirm the channel-move path and check the empty-filter guard.


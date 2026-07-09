---
type: research-record
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
source_lines: 3811-3839
agent_attribution: subagent (coherence audit agent)
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Coherence audit of three concurrent features (MCP, TUI, mgmt) against the redesign — verdicts: coherent with one vocabulary fix, fully coherent, coherent; flagged 3 items for human review"
captured_at: 2026-07-09T14:55:46Z
---

Coherence audit of three concurrent features (MCP, TUI, mgmt) against the redesign — verdicts: coherent with one vocabulary fix, fully coherent, coherent; flagged 3 items for human review

---

User: [Agent task result: Agent "Coherence audit of concurrent work" finished]
Coherence audit complete. All green gates pass and changes are uncommitted in `/Users/pablofernandez/src/tenex-edge-coherence` (branch `fix/coherence-concurrent`).

## Coherence report

### Feature 1 — MCP HTTP server (`src/cli/mcp/**`)
**Verdict: coherent, with one minor vocabulary drift (fixed).**

Full catalog audited (`catalog.rs` SPECS + `tools.rs` dispatch): the exposed tools are `who`, `channels_list`, `chat_read`, `chat_write`, `channels_create`, `channels_join`, `channels_leave`, `channels_switch`.
- **No removed surface exposed.** There is no `invite` tool, no `project init/edit` tool. Nothing promises removed behavior. `resources.rs` (channel-status/roster/fabric) and `protocol.rs` are clean.
- **Vocabulary drift (fixed):** `chat_write` description was "Write a message to channel chat." — "Write" is not the shipped verb; the redesign ships `channel send`. Changed to **"Send a message to a channel."** For parallelism I also changed `chat_read` from "Read recent channel chat." to **"Read recent messages from a channel."** (`src/cli/mcp/catalog.rs:55,69`). Tool public names and RPC dispatch (`chat_read`/`chat_write` methods) are unchanged, so nothing breaks.
- **Note (not drift):** the shipped CLI itself still uses "channel chat" freely (e.g. `channel read` = "Read channel chat history"), so "chat" is NOT purged vocabulary — only the verb `send` needed alignment.
- **`channels_create` agents param** ("slug@backend strings") stays — it matches `crate::idref::parse_agent_backend_ref` and the codebase's own idref error vocabulary; renaming to "role@machine" would diverge from the actual parser.

### Feature 2 — TUI live session browser (`src/cli/tui/**`)
**Verdict: fully coherent as-is. No changes.**
Renders `agent` slug + title/activity + channels + PTY + cwd + command; keybind hints are attach/pane/kill/refresh/quit. Grep for `invite|ordinal|instance|project init|role slug|chat` across the module returns nothing. It never labels anything "ordinal", never exposes a removed command, and shows the local-view identity (agent slug + session id) which is the correct local model. The fabric `@codename@host` handle is not surfaced, which is fine per the task brief (local browser, not fabric addressing).

### Feature 3 — mgmt chat commands (`src/daemon/server/management_command*`)
**Verdict: coherent. No changes.**
- **`add` (`parse.rs:13`, `management_command.rs:112`):** parses `slug[@backend]` and spawns via `invite_rpc::invite_agent` (RPC name intentionally unchanged). This is the `channel add --new-session <role>[@machine]` path — spawning a fresh session by role, consistent with the new model (spawn ≠ addressing a live session by role). User-facing reply says "added … to <channel>" (not "invited"). Coherent.
- **`list agents` vs `list sessions`:** `list_agents.rs` reads `list_agent_roster()` keyed by role slug + use-criteria = the advertised **role-type roster**; `sessions.rs` lists live **sessions** (members). The roster=role-types / members=sessions distinction is correct.

## What I flagged for human review (implemented conservative option: left as-is)
1. **MCP tool rename `chat_read→channel_read`, `chat_write→channel_send`:** technically safe (tool name maps to RPC method in `tools.rs`, so RPC dispatch wouldn't break), and more aligned with shipped `channel read`/`channel send`. Not done because it's a client-facing breaking change to already-published MCP tool names; per instructions I kept names and fixed descriptions only.
2. **No MCP `channel add` / recruit tool exists.** The existing catalog is coherent, but there's no MCP equivalent of the shipped `channel add` (human / `--new-session` / `--session`) despite `channels_create` accepting an `agents` array. Adding one is net-new surface + RPC wiring, so I did not add it — flagging as a possible follow-up.
3. **mgmt `add` covers only the new-session path** (`slug[@backend]`), not `channel add`'s human-add or `--session @codename@host` existing-session modes. This is a coherent subset for a chat command, not an incoherence; noted in case parity is desired.

Diff: 2 lines in `src/cli/mcp/catalog.rs`. Gates: `cargo build` OK, `just fmt-check` OK, `just lint` OK, `just loc-check` OK, `just test-unit` 877 passed / 0 failed. Uncommitted.

---
type: research-record
date: 2026-06-28
session: b9176726-a9a8-41a9-b806-c966e8c94ed7
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b9176726-a9a8-41a9-b806-c966e8c94ed7.jsonl
source_lines: 2630-2659
agent_attribution: main
has_preregistered_criteria: true
has_method: true
has_structured_report: true
characterization: "Two-bug analysis: empty relay filter REQs from old binary (fixed in rewrite) and ordinal allocation putting same agent across multiple channels in new binary"
captured_at: 2026-06-28T15:33:03Z
---

Two-bug analysis: empty relay filter REQs from old binary (fixed in rewrite) and ordinal allocation putting same agent across multiple channels in new binary

---

Assistant: I owe you a correction: I was wrong twice, and the live new-schema data proves it. Here's the accurate picture.

## Bug 1 — the empty `#d:[""]`/`#h:[""]` REQs: **old binary, already gone in the rewrite**
Those REQs use sub-names (`mux_chat_`, `meta_`, `roles_`, kind `39003`) that **don't exist anywhere in the new code** — they're the pre-rewrite daemon. The new filter builders (`subscriptions.rs:74-107`) omit the `#h`/`#d`/`#p` tag entirely when coverage is empty (kinds-only filter), and the caller skips opening that REQ. So the new code never emits `[""]`. (One latent wart: the empty case yields an *unscoped* kinds-only filter that relies on the caller's skip — worth tightening, but not the `[""]` bug.)

## Bug 2 — same agent on multiple channels: **real, live in the new binary**
My "structurally impossible" claim was too narrow and wrong. The live `/tmp/te` DB (new schema) shows:

| session | channel_h | alive | pid | pane | harness id |
|---|---|---|---|---|---|
| te-18bd3e9 | test1-c028b76a | **1** | 21103 | %58 | 4054e0c2 |
| te-18bd3ea | test1 | 0 | 42172 | %59 | 134de40a |
| te-18bd3ee | omnigent | **1** | 50236 | — | d1e5cdd5 |

**Two alive rows, same base pubkey `4077e280`, two channels, both heartbeating** (last_seen 2s apart). Each *row* has one `channel_h` (that part of my claim holds) — but the **same agent identity fragments into N rows**, one per spawn.

### Root cause (verified)
- `register_session` keys identity only on the harness-native id. The harness mints a *fresh* id per spawn, so each orchestration spawn into a new channel = new alias = **new minted canonical session**.
- The only de-dup is `session_start.rs:212` and it fires solely when `same_agent_slug && same_work_root && (same_pid || same_pane)`. These spawns have **different pids and panes** → predicate never matches → all rows stay alive.
- The proper `resolve_identity` (Mint/Reattach/**Supersede**) logic exists in `session.rs:394` but is **dead code** — only called from tests, never wired into `register_session`.
- Deeper: all three share **ordinal-0 (the same pubkey)**. Genuinely-concurrent instances were supposed to get distinct ordinal pubkeys; they didn't, so one identity is live in multiple channels — exactly the "no single source of truth for channel id" you called out.

## The fix needs your design call
"Where is agent X" can only have one answer if we decide what that means:

**A — one identity, one channel at a time:** spawning/moving an agent into a new channel **supersedes** its prior session (mark dead, stop its heartbeat) regardless of pid/pane. Channel becomes the single mutable truth on one row.

**B — concurrent instances allowed, but never on the same key:** genuinely-concurrent sessions of one agent must get **distinct ordinal pubkeys**, so no base pubkey is ever in two channels; the ordinal allocator is what's broken.

Which model is correct — is an agent allowed to be live in multiple channels simultaneously at all? That determines whether the fix is "supersede on spawn" or "fix ordinal allocation," and I'm not going to guess a third time.

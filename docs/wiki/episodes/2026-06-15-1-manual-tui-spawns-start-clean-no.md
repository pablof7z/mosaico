---
type: episode-card
date: 2026-06-15
session: 622711fa-5176-4580-b311-d66446c2924b
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/622711fa-5176-4580-b311-d66446c2924b.jsonl
salience: product
status: active
subjects:
  - tmux-spawn-prompt-contract
  - pending-spawn-injection
supersedes:
  - 2026-06-15-3-eliminate-inbox-prompt-injection-on-manual
related_claims: []
source_lines:
  - 3-5
  - 836-865
captured_at: 2026-06-15T07:11:20Z
---

# Episode: Manual TUI spawns start clean — no inbox injection

## Prior State

Every manual TUI spawn registered SPAWN_PROMPT_DEFAULT ("tenex-edge inbox") as a PendingSpawn; rpc_session_start would inject it as the first user message ~2s after session creation, regardless of whether any messages existed

## Trigger

User reported new sessions auto-send 'tenex-edge inbox' as a user message after a few seconds and said it shouldn't send ANYTHING

## Decision

Removed SPAWN_PROMPT_DEFAULT entirely. Manual TUI spawns now register no PendingSpawn, so rpc_session_start injects nothing. Only spawn-on-send (inbound mention triggers the spawn) gets the actual mention injected as the first prompt. The doorbell mechanism remains for sessions that genuinely have unread inbox rows.

## Consequences

- Manually spawned sessions start with a clean prompt — no injected command
- Spawn-on-send still works: the triggering mention is typed into the new session via register_pending_spawn_with_mention
- Doorbell injection unchanged: only fires when a session has real unread inbox rows and is idle

## Open Tail

*(none)*

## Evidence

- transcript lines 3-5
- transcript lines 836-865


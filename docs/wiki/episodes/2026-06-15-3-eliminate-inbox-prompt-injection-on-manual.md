---
type: episode-card
date: 2026-06-15
session: 622711fa-5176-4580-b311-d66446c2924b
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/622711fa-5176-4580-b311-d66446c2924b.jsonl
salience: architecture
status: superseded
subjects:
  - tenex-edge-spawn
  - inbox-injection
  - pending-spawn
supersedes:
  - 2026-06-14-1-spawn-prompt-should-inject-actual-mention
related_claims: []
source_lines:
  - 3-5
  - 636-666
  - 836-849
captured_at: 2026-06-15T07:05:43Z
---

# Episode: Eliminate inbox prompt injection on manual TUI spawns

## Prior State

When a new session was created via the TUI, the system injected the text "tenex-edge inbox" as a user message into the pane after a short delay (SPAWN_PROMPT_DEFAULT). Manual TUI spawns registered a PendingSpawn entry that caused rpc_session_start to inject this default prompt.

## Trigger

User directive: new TUI-spawned sessions should send NOTHING as user input — no inbox command, no prompt at all.

## Decision

Removed SPAWN_PROMPT_DEFAULT constant entirely. Manual TUI spawns no longer register a PendingSpawn, so rpc_session_start injects nothing into the pane. Only spawn-on-send (mention-triggered) spawns register a PendingSpawn with the triggering mention, which rpc_session_start then types as the session's first prompt. The doorbell mechanism remains for genuinely unread messages.

## Consequences

- Manual TUI spawns start completely clean — no injected text
- Spawn-on-send path unchanged: the actual received mention is typed into the new session as its first prompt
- Clear architectural split: PendingSpawn now exclusively means spawn-on-send with a real message payload
- Doorbell nudge still fires independently for sessions with unread inbox rows

## Open Tail

*(none)*

## Evidence

- transcript lines 3-5
- transcript lines 636-666
- transcript lines 836-849


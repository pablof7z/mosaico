---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: root-cause
status: superseded
subjects:
  - session-spawn-race
  - title-seed-lag
  - distill-timing
supersedes: []
related_claims: []
source_lines:
  - 571-616
  - 838-844
  - 894-951
  - 952-975
captured_at: 2026-06-16T11:33:11Z
---

# Episode: Three session-state bugs diagnosed and patched (interim)

## Prior State

spawn_session was non-atomic (check and insert separated by .await points), turn_start received no prompt text (only the transcript path, which Claude hadn't flushed yet), and turn_first defaulted to 30s—meaning the LLM distiller was never scheduled before most turns ended

## Trigger

User observed flip-flopping titles on the same d-tag (two zombie runtimes), first message showing no title then lagging one message behind, and LLM title never generated at all

## Decision

Three patches: (1) atomic check-and-reserve in spawn_session before any .await, so second spawn returns early; (2) thread prompt text through user-prompt-submit → turn_start → rpc_turn_start → persist as last_user_prompt, seed from it directly instead of lagging transcript; (3) lower turn_first default from 30s to 3s so the first obs tick schedules distill

## Consequences

- Zombie runtime flip-flop stopped for new sessions; existing orphans require daemon restart
- Title now appears on first message instead of second
- LLM distillation actually fires within a turn
- All three are interim patches—the full session-state refactor will supersede them with a single authoritative row

## Open Tail

- Three patches uncommitted in working tree; user chose to fold into the refactor branch rather than ship separately

## Evidence

- transcript lines 571-616
- transcript lines 838-844
- transcript lines 894-951
- transcript lines 952-975


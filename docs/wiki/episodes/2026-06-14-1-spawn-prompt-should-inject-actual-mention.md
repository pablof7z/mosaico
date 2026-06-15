---
type: episode-card
date: 2026-06-14
session: 0afc3cf4-3465-4b37-a7ec-63b798d78621
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/0afc3cf4-3465-4b37-a7ec-63b798d78621.jsonl
salience: product
status: superseded
subjects:
  - tmux-spawn-prompt
  - pending-spawn-default
supersedes: []
related_claims: []
source_lines:
  - 1-1
  - 38-48
  - 50-50
  - 54-67
captured_at: 2026-06-14T19:00:21Z
---

# Episode: Spawn prompt should inject actual mention content, not generic default

## Prior State

Newly spawned tmux sessions receive SPAWN_PROMPT_DEFAULT ("tenex-edge inbox") injected via `tmux send-keys` after a 2-second delay, regardless of which mention triggered the spawn. The prompt is treated as a mere trigger to start a turn, not as meaningful content for the agent.

## Trigger

User observed the generic prompt and explicitly rejected it: when a session is spawned because an agent was p-tagged in a message, the injected prompt should contain the actual message content, not a meaningless generic command.

## Decision

The spawn prompt for mention-triggered sessions must convey the actual triggering message content rather than the opaque default. The `register_pending_spawn_with_mention` path already carries the mention data; it should drive the prompt text instead of falling back to `SPAWN_PROMPT_DEFAULT`.

## Consequences

- `SPAWN_PROMPT_DEFAULT` is a design flaw for the mention-triggered spawn path — it shows the agent a command with no semantic relevance to why it was started
- Reply threading via `mention_event_id` must be preserved in whatever prompt format replaces the default
- The turn-start inbox drain still delivers actual content; the prompt change is about what the agent sees typed into its input, not about the inbox delivery mechanism

## Open Tail

- Exact format of the mention-derived prompt (render the full envelope? just the body? a `tenex-edge inbox` plus context?)
- Whether `SPAWN_PROMPT_DEFAULT` remains as fallback for non-mention spawns (e.g. manual `tmux spawn`)

## Evidence

- transcript lines 1-1
- transcript lines 38-48
- transcript lines 50-50
- transcript lines 54-67


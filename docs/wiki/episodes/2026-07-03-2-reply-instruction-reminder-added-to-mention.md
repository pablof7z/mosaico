---
type: episode-card
date: 2026-07-03
session: e0eba763-d227-40ca-a9d2-aaad5b192130
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/e0eba763-d227-40ca-a9d2-aaad5b192130.jsonl
salience: product
status: active
subjects:
  - mention-injection
  - reply-hint
  - render-tmux-mention
  - render-messages
  - tenex-edge-chat-write
supersedes: []
related_claims: []
source_lines:
  - 1-3
  - 109-173
captured_at: 2026-07-03T10:33:06Z
---

# Episode: Reply-instruction reminder added to mention injection paths

## Prior State

Both mention injection paths (hook-only fabric context rendering and tmux-wrapped injection) deliberately included no reply hint. The doc comment in injection.rs explicitly stated 'No reply hint, no message id — the reply auto-publishes', because replies were expected to be auto-captured by the publish_agent_reply flow.

## Trigger

User directive: 'when we bring in a mention into an agent's attention (whether via hook or via tmux-wrapped injection), let's explicitly add an instruction reminding the agent to respond via tenex-edge chat write.' This is also a consequence of removing auto-publish — without auto-publish, agents need to know how to reply explicitly.

## Decision

Add an explicit reply-instruction reminder (referencing 'tenex-edge chat write') to both mention injection paths: the hook-only path in render.rs (render_messages, around the [MENTIONS YOU] marker at line 146-147) and the tmux-wrapped path in injection.rs (render_tmux_mention, around lines 56-65).

## Consequences

- render_messages in fabric_context/render.rs must append a reply hint near the [MENTIONS YOU] flag
- render_tmux_mention in injection.rs must append a reply instruction to the joined output
- The canonical command name is 'tenex-edge chat write' (confirmed by existing usage in who/render.rs and channel_membership_rpc.rs)
- The doc comment in injection.rs stating 'No reply hint' must be updated to reflect the new behavior

## Open Tail

- Exact wording and placement of the reply hint in each path
- Whether the hint should appear on every mention or only when the agent is expected to reply

## Evidence

- transcript lines 1-3
- transcript lines 109-173

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-03-2-reply-instruction-reminder-added-to-mention.json`](transcripts/2026-07-03-2-reply-instruction-reminder-added-to-mention.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-03-2-reply-instruction-reminder-added-to-mention.json`](transcripts/raw/2026-07-03-2-reply-instruction-reminder-added-to-mention.json)

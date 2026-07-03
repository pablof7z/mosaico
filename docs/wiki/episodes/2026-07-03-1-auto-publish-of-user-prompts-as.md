---
type: episode-card
date: 2026-07-03
session: e0eba763-d227-40ca-a9d2-aaad5b192130
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/e0eba763-d227-40ca-a9d2-aaad5b192130.jsonl
salience: reversal
status: active
subjects:
  - auto-publish-kind9
  - user-prompt-submit-hook
  - rpc-user-prompt
  - publish-agent-reply
supersedes: []
related_claims: []
source_lines:
  - 1-3
  - 67-108
captured_at: 2026-07-03T10:33:06Z
---

# Episode: Auto-publish of user prompts as kind:9 removed in favor of explicit publishing

## Prior State

User prompt submission automatically triggered publishing the prompt as a kind:9 chat event (operator-signed) via rpc_user_prompt in chat_publish.rs. A symmetric flow auto-published agent turn output as kind:9 on Stop. The design intent was that replies auto-publish, so no explicit publish step was needed.

## Trigger

User explicitly reversed a prior design decision: 'I've changed my mind about the user hook auto-publishing kind:9s — nothing should do that — let's leave it as something that agents do explicitly or users do explicitly.'

## Decision

Auto-publishing of user prompts as kind:9 on the user-prompt-submit hook is to be removed/disabled. Publishing should become an explicit action by agents or users, not an automatic side-effect of prompt submission. The symmetric agent-reply auto-publish (publish_agent_reply in chat_publish.rs via turns.rs) is also relevant for symmetric removal.

## Consequences

- rpc_user_prompt (chat_publish.rs:83-211) and its call site in hooks.rs:388-411 must be removed or disabled
- publish_agent_reply (chat_publish.rs:42-81) invoked from turns.rs:210-223 is a candidate for symmetric removal
- Agents and users must now explicitly publish chat events rather than relying on automatic capture
- Removal of auto-publish creates a need for explicit reply instructions (see related arc on reply-hint injection)

## Open Tail

- Whether to symmetrically remove agent-reply auto-publish or only the user-prompt side
- Whether any existing sessions/workflows depend on auto-published kind:9 events for state coherence

## Evidence

- transcript lines 1-3
- transcript lines 67-108

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-03-1-auto-publish-of-user-prompts-as.json`](transcripts/2026-07-03-1-auto-publish-of-user-prompts-as.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-03-1-auto-publish-of-user-prompts-as.json`](transcripts/raw/2026-07-03-1-auto-publish-of-user-prompts-as.json)

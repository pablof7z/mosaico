---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: product
status: superseded
subjects:
  - whitelisted-operator
  - sender-label
  - chat-row-refs
  - host-rendering
supersedes: []
related_claims: []
source_lines:
  - 168-173
  - 175-181
  - 693-722
  - 1112-1115
captured_at: 2026-07-09T14:42:06Z
---

# Episode: Whitelisted operator sender label rendered bare without host

## Prior State

Whitelisted human operators (in `whitelisted_pubkeys` config) have no session or host. In the channel-read path, `chat_row_refs` fell through to `unwrap_or("?")` producing `<Pablo@?>`. In the fabric_context path, `pubkey_ref` misattributed them to the viewer's own `local_host`. The `is_whitelisted` concept already existed in `injection.rs` and rendered terminal-injected mentions as bare `<@name>`, but `chat_row_refs` and `pubkey_ref` never checked it.

## Trigger

User spotted `<Pablo@?>` in the channel read output mid-session, identifying it as a rendering bug.

## Decision

`chat_row_refs` now checks `whitelisted_pubkeys` for the author pubkey and short-circuits to an empty/omitted host. The CLI renderer (`render_chat_read_row` in `src/cli/messaging.rs`) now prints `<@name>` instead of `<name@?>` when host is empty, mirroring the bare `<@name>` convention from `injection.rs`.

## Consequences

- Whitelisted operators now render as `<@Pablo>` in channel reads, consistent with terminal-injected mention rendering.
- The `?` placeholder fallback for host is eliminated for whitelisted pubkeys; the bare-slug convention is unified across rendering surfaces.
- Regression test `chat_read_row_renders_hostless_sender_bare_not_with_question_mark` added.

## Open Tail

- The fabric_context path (`pubkey_ref` in `refs.rs`) still misattributes whitelisted operators to `local_host` — the fix direction was identified but implementation in this session focused on the CLI/server path.

## Evidence

- transcript lines 168-173
- transcript lines 175-181
- transcript lines 693-722
- transcript lines 1112-1115

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-rendered-bare.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-rendered-bare.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-rendered-bare.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-rendered-bare.json)

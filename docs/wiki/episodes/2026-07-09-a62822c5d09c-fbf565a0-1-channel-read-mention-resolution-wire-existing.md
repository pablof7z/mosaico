---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: root-cause
status: active
subjects:
  - chat-read-mention-resolution
  - rewrite-body-mentions
supersedes:
  - 2026-07-09-a62822c5d09c-fbf565a0-1-channel-read-mention-resolution-wired-into
related_claims: []
source_lines:
  - 57-74
  - 1104-1107
captured_at: 2026-07-09T14:50:00Z
---

# Episode: Channel-read mention resolution: wire existing resolver into chat_read path

## Prior State

The profile mention resolver (`rewrite_body_mentions`) existed and was correctly invoked on the fabric-context/snapshot paths (messages.rs, capture/read.rs, turn_context/reads.rs) but was never called in the `chat_read_tail` server-side JSON builder. Raw `nostr:npub1…` strings passed through verbatim to the CLI renderer.

## Trigger

User observed unresolved npub mentions when running `tenex-edge channel read` — names showed as raw bech32 tokens instead of @-mentions.

## Decision

Added `rewrite_body_mentions` (plus `body_mention_pubkeys` cache pre-warm) calls in `chat_row_to_json`/`chat_log_row_to_json` in `chat_read_tail.rs`, mirroring the pattern already used in `fabric_context/messages.rs`. Fix is server-side so the CLI renderer inherits it automatically.

## Consequences

- All `channel read` output now resolves `nostr:npub…`/`nostr:nprofile…` tokens to `@<name>` (or pubkey-short fallback), consistent with the hook-injected snapshot path.
- Single source of truth for mention resolution is now uniformly applied across all chat-rendering paths.

## Open Tail

*(none)*

## Evidence

- transcript lines 57-74
- transcript lines 1104-1107

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-1-channel-read-mention-resolution-wire-existing.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-1-channel-read-mention-resolution-wire-existing.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-1-channel-read-mention-resolution-wire-existing.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-1-channel-read-mention-resolution-wire-existing.json)

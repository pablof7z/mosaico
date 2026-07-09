---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: product
status: active
subjects:
  - chat-read-rendering
  - whitelisted-pubkeys
  - host-rendering
supersedes:
  - 2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-bare-name
related_claims: []
source_lines:
  - 1112-1113
captured_at: 2026-07-09T18:32:48Z
---

# Episode: Whitelisted human operators render as bare @name instead of name@?

## Prior State

Whitelisted human operators (`whitelisted_pubkeys` in config) have no session/host association, so `chat_row_refs` fell through to a `?` placeholder for the host field, rendering as `<Pablo@?>` in channel read output.

## Trigger

User caught the `<Pablo@?>` rendering bug mid-fix while reviewing channel read output.

## Decision

`chat_row_refs` now short-circuits to a bare (empty) host for whitelisted pubkeys, and the CLI renderer (`messaging.rs`) prints `<@Pablo>` instead of `<Pablo@?>` when host is empty — mirroring the bare `<@name>` convention already used for terminal-injected mentions in `injection.rs`.

## Consequences

- Whitelisted humans render consistently with the bare @name convention used elsewhere in the system
- Regression test added for whitelisted-host bare rendering

## Open Tail

*(none)*

## Evidence

- transcript lines 1112-1113

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-human-operators-render-as-bare.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-human-operators-render-as-bare.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-human-operators-render-as-bare.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-human-operators-render-as-bare.json)

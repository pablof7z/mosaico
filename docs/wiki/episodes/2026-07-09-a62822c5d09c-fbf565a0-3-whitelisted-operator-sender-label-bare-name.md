---
type: episode-card
date: 2026-07-09
session: a62822c5-d09c-4a83-9251-a3856d276ac4
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/a62822c5-d09c-4a83-9251-a3856d276ac4.jsonl
salience: product
status: active
subjects:
  - whitelisted-operator-rendering
  - chat-row-refs
supersedes:
  - 2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-rendered-bare
related_claims: []
source_lines:
  - 1112-1113
captured_at: 2026-07-09T14:50:00Z
---

# Episode: Whitelisted operator sender label: bare <@name> instead of <name@?>

## Prior State

Whitelisted human operators (in `whitelisted_pubkeys` config) have no session or host, so `chat_row_refs` fell through to a `?` placeholder for the host field, producing labels like `<Pablo@?>`.

## Trigger

User caught the `<Pablo@?>` rendering mid-session while looking at the transcript output.

## Decision

`chat_row_refs` now short-circuits to a bare (empty) host for whitelisted pubkeys. The CLI renderer (`messaging.rs`) prints `<@Pablo>` instead of `<Pablo@?>` when host is empty, mirroring the bare `<@name>` convention already used for terminal-injected mentions in `injection.rs`.

## Consequences

- Whitelisted human operators are rendered as `<@name>` consistently across CLI and hook-injection paths.
- Eliminates the confusing `?` placeholder for any participant without a host assignment.

## Open Tail

*(none)*

## Evidence

- transcript lines 1112-1113

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-bare-name.json`](transcripts/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-bare-name.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-bare-name.json`](transcripts/raw/2026-07-09-a62822c5d09c-fbf565a0-3-whitelisted-operator-sender-label-bare-name.json)

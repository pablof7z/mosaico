---
type: episode-card
date: 2026-06-28
session: b07a57a3-67a1-4c44-a8fc-58a1bb97860a
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b07a57a3-67a1-4c44-a8fc-58a1bb97860a.jsonl
salience: product
status: active
subjects:
  - cli-launch
  - channel-provisioning
supersedes: []
related_claims: []
source_lines:
  - 1-110
  - 268-289
captured_at: 2026-06-28T12:11:50Z
---

# Episode: Interactive channel creation prompt for unknown `--channel` names

## Prior State

When `tenex-edge launch <agent> --channel <name>` is invoked, the name is treated as a literal NIP-29 group h-value and passed to the relay. If no such group exists, nothing is created and the user receives no feedback.

## Trigger

User explicit directive (line 110): when a channel name is unknown, the system should prompt the user to create it instead of silently failing. User notes that channel state is already available locally to distinguish known vs unknown channels.

## Decision

Change `launch --channel` behavior to check provided names against channels_list; if unknown, prompt the user to create the channel instead of treating it as an h-value lookup.

## Consequences

- Replaces silent failure with guided user interaction for channel creation
- Requires name-to-h-value resolution and unknown-name detection logic
- CLI must distinguish between valid h-value format and potential channel names
- Unblocks two-step manual workflow (create first, then launch)

## Open Tail

- Relay connectivity bug discovered during investigation: daemon reports 'relay not connected' on every create-group attempt. Root cause not identified; may involve Transport layer, async startup sequencing, NIP-42 AUTH, or configuration.
- UX design decision needed: reuse interactive picker modal or create new confirmation flow?
- Implementation is blocked until relay connectivity during daemon startup is fixed

## Evidence

- transcript lines 1-110
- transcript lines 268-289

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-06-28-1-interactive-channel-creation-prompt-for-unknown.json`](transcripts/2026-06-28-1-interactive-channel-creation-prompt-for-unknown.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-06-28-1-interactive-channel-creation-prompt-for-unknown.json`](transcripts/raw/2026-06-28-1-interactive-channel-creation-prompt-for-unknown.json)

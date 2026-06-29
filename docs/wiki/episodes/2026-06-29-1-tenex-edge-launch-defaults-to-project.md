---
type: episode-card
date: 2026-06-29
session: 76eb2229-ffbb-4b24-84b9-7d9caabc93ca
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/76eb2229-ffbb-4b24-84b9-7d9caabc93ca.jsonl
salience: product
status: active
subjects:
  - tenex-edge-launch
  - channel-resolution
  - cli-defaults
supersedes: []
related_claims: []
source_lines:
  - 1-2
  - 112-119
captured_at: 2026-06-29T11:11:55Z
---

# Episode: tenex-edge launch defaults to project channel when no --channel given

## Prior State

Running `tenex-edge launch <agent>` without a `--channel` flag opened the interactive channel picker (when per_session_rooms=false, the default).

## Trigger

User directive: 'if I use tenex-edge launch with no channel it should default to the project channel.'

## Decision

Omitting `--channel` now defaults to the project root channel. The interactive picker is only invoked by an explicit `--channel ""` (empty string).

## Consequences

- Users get a sensible default channel without extra flags, reducing friction for the common launch path.
- The interactive picker is still reachable but now requires an explicit empty-string opt-in, changing the ergonomic contract of the flag.
- Existing scripts or muscle-memory that relied on no-`--channel` opening the picker will need updating to pass `--channel ""`.

## Open Tail

- Behavior when per_session_rooms=true is not discussed in this session — the default-to-project-channel change may only apply to the per_session_rooms=false path.

## Evidence

- transcript lines 1-2
- transcript lines 112-119

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-06-29-1-tenex-edge-launch-defaults-to-project.json`](transcripts/2026-06-29-1-tenex-edge-launch-defaults-to-project.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-06-29-1-tenex-edge-launch-defaults-to-project.json`](transcripts/raw/2026-06-29-1-tenex-edge-launch-defaults-to-project.json)

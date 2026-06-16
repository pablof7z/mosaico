---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: root-cause
status: active
subjects:
  - distill-title-seed
  - turn-start-hook
supersedes:
  - 2026-06-16-1-session-title-one-turn-behind-seed
related_claims: []
source_lines:
  - 5-26
  - 1016-1021
captured_at: 2026-06-16T12:05:20Z
---

# Episode: Title one-turn-behind: seed must capture prompt verbatim, not read transcript

## Prior State

The distiller seeded its title from the transcript file, which was not yet flushed when Claude wrote the new prompt. turn_start only received the transcript path, so the first distill always operated on stale content, producing a title that lagged one message behind.

## Trigger

Tracing the title-publish flow revealed that user-prompt-submit had the prompt text but only used it for the kind:1 publish; turn_start got only the path and the seed read read_last_user_prompt(transcript) before the harness had flushed.

## Decision

Capture the prompt verbatim at turn-start and persist it in a new last_user_prompt column; the distiller seeds from the captured prompt directly, falling back to transcript only if empty.

## Consequences

- New DB column last_user_prompt + set_/get accessors on Store
- Hooks and turn_start thread the prompt through and persist it
- Interim fix — subsumed by versioned session_state transitions in the upcoming refactor

## Open Tail

*(none)*

## Evidence

- transcript lines 5-26
- transcript lines 1016-1021


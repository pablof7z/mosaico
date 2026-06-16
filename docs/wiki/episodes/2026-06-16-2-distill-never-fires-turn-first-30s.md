---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: root-cause
status: active
subjects:
  - distill-scheduling
  - obs-loop-timing
supersedes:
  - 2026-06-16-2-distill-timing-bug-title-never-generated
related_claims: []
source_lines:
  - 954-973
  - 1022-1023
captured_at: 2026-06-16T12:05:20Z
---

# Episode: Distill never fires: turn_first=30s exceeds most turn durations

## Prior State

The obs loop ticks every 5s and checks whether a distill is due. turn_first default was 30s, meaning the first distill opportunity wouldn't arise until 30s into a turn — most turns finish sooner, so the distiller was never scheduled and no LLM title was ever generated.

## Trigger

Investigating why the LLM title was never generated revealed no error log because the distill task simply never ran; the 30s gate made it impossible within typical turn durations.

## Decision

Lower turn_first default from 30s to 3s, below the 5s obs interval, so the first obs tick of any multi-second turn schedules the distill.

## Consequences

- Distill now fires within the first obs tick of a multi-second turn
- Interim fix — the upcoming stateless SessionDriver expresses timing as explicit transition logic instead of scattered timer constants

## Open Tail

*(none)*

## Evidence

- transcript lines 954-973
- transcript lines 1022-1023


---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: root-cause
status: superseded
subjects:
  - distill-timing
  - turn-first
  - obs-interval
supersedes: []
related_claims: []
source_lines:
  - 842-843
  - 952-975
captured_at: 2026-06-16T11:42:46Z
---

# Episode: Distill-timing bug — title never generated

## Prior State

turn_first default was 30 seconds; the obs loop ticks every 5s checking 'is a distill due?'; most Claude turns finish before 30s, so the distiller was never scheduled and no LLM title was ever produced — with no error log because it never ran

## Trigger

Diagnosis: the distiller simply never fires within typical turn duration because the scheduling gate is higher than the turn lifetime

## Decision

Lower turn_first default from 30s to 3s, below the 5s obs interval, so the first obs tick of any multi-second turn schedules the distill

## Consequences

- LLM title distiller now fires within the first obs tick of a turn
- This constant will become a property of the session_state transition scheduler in the rearchitecture

## Open Tail

- Subsumed by the stateless runtime driver's on_tick scheduling

## Evidence

- transcript lines 842-843
- transcript lines 952-975


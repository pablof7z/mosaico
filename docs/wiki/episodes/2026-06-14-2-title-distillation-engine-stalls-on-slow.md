---
type: episode-card
date: 2026-06-14
session: 215d979a-a054-4e2b-b349-851e0d874d6d
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/215d979a-a054-4e2b-b349-851e0d874d6d.jsonl
salience: root-cause
status: superseded
subjects:
  - title-distillation
  - engine-distill
  - complete-via-rig
supersedes: []
related_claims: []
source_lines:
  - 137-1116
captured_at: 2026-06-14T19:18:31Z
---

# Episode: Title distillation engine stalls on slow/failing API calls

## Prior State

Assumption that session titles publish promptly within ~30 seconds of turn start via the distillation engine

## Trigger

This session ran 15+ minutes with no title appearing in the tmux session list, which the user flagged as a critical visibility failure

## Decision

Identified two root causes: (1) `complete_via_rig` has no timeout — a slow OpenRouter API call blocks the entire engine (no heartbeats, no presence, no turn-end detection) for 6+ minutes; (2) `last_distill = now` is set unconditionally even when `distill_session` returns `None`, consuming the single retry slot when `turn_repeat=0`

## Consequences

- Engine freezes during slow distillation API calls — all runtime tasks stall
- Failed distillation permanently loses the title for that turn (no retry possible with turn_repeat=0)
- Title was only published at 22:09:27 (6+ min into turn 2) after a blocked API call finally returned

## Open Tail

- Fix 1 proposed but not yet implemented: wrap `complete_via_rig` in a timeout
- Fix 2 proposed but not yet implemented: only set `last_distill = now` on successful distillation

## Evidence

- transcript lines 137-1116


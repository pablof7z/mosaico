---
type: episode-card
date: 2026-06-16
session: 1b868736-ed6b-4f88-84d9-26bb320accfd
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1b868736-ed6b-4f88-84d9-26bb320accfd.jsonl
salience: root-cause
status: superseded
subjects:
  - title-seed
  - turn-start
  - last-user-prompt
supersedes:
  - 2026-06-16-1-three-session-state-bugs-diagnosed-and
related_claims: []
source_lines:
  - 838-844
  - 894-951
  - 979-1012
captured_at: 2026-06-16T11:42:46Z
---

# Episode: Session title one-turn-behind seed fix

## Prior State

turn_start received only the transcript file path; the runtime seed called read_last_user_prompt on that file before Claude had flushed the just-submitted prompt to disk, so turn 1 always saw an empty seed and turn N saw the now-flushed message N-1

## Trigger

Diagnosis: first message produces no title, subsequent titles lag one turn behind because the transcript read races the harness file write

## Decision

Thread the prompt text directly through the hook→turn_start→rpc pipeline; add a last_user_prompt column + setter/getter to state.rs; the runtime seed prefers the captured prompt and falls back to the transcript only if empty

## Consequences

- First turn now has a real title seed instead of blank
- Adds a schema column (last_user_prompt TEXT) to the sessions table
- This fix will be subsumed by the versioned session_state transition apply_distill_result (which stores turn_id+version and ignores stale results)

## Open Tail

- Subsumed by the full session aggregate rearchitecture

## Evidence

- transcript lines 838-844
- transcript lines 894-951
- transcript lines 979-1012


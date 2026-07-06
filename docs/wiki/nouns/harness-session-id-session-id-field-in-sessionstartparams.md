---
type: noun-entry
slug: harness-session-id-session-id-field-in-sessionstartparams
name: "harness_session_id (session_id field in SessionStartParams)"
origin: extracted
source_refs:
  - transcript:120-123
  - transcript:173-175
---

# harness_session_id (session_id field in SessionStartParams)

The harness-native external session id sent by hooks; it is ONLY a locator for `session_aliases`, never the identity. It is Some for harnesses that own an id (claude-code, codex) and None for programmatic hosts (opencode) whose stable anchors are the resume token / PTY session / watched pid.

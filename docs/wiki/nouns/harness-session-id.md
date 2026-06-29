---
type: noun-entry
slug: harness-session-id
name: "harness_session_id"
origin: extracted
source_refs:
  - transcript:120-123
  - transcript:172-174
---

# harness_session_id

The harness-owned external session id, present only for harnesses that own an id of their own (claude-code, codex); None for programmatic hosts (opencode). It is ONLY a locator for session_aliases, never the identity — the daemon resolves the canonical id.

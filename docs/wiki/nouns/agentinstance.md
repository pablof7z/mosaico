---
type: noun-entry
slug: agentinstance
name: "AgentInstance"
origin: extracted
source_refs:
  - transcript:921-928
---

# AgentInstance

Superseded by per-session identity. There is no base-vs-ordinal identity policy
to carry: each session has exactly one keypair, derived from the machine's
management key as `derive(management_secret, session_id)`. The session id and its
derived pubkey are the whole identity; there is no base pubkey or ordinal. See
[Tenex-Edge Agent Identity](../guides/tenex-edge-agent-identity.md).

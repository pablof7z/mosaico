---
type: noun-entry
slug: agentidentity
name: "AgentIdentity"
origin: extracted
source_refs:
  - transcript:102-105
---

# AgentIdentity

Superseded by per-session identity. A session's signing key is derived from the
machine's management key as `derive(management_secret, session_id)`, not resolved
from a durable per-agent keypair. `--agent <slug>` now names a role config
(`<edge_home>/agents/<slug>.json`: harness, provider, model), not a stored
identity key. See [Tenex-Edge Agent Identity](../guides/tenex-edge-agent-identity.md).

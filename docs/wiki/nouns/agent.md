---
type: noun-entry
slug: agent
name: "agent"
origin: extracted
source_refs:
  - transcript:359-362
---

# agent

A role that a session plays on the fabric (defined by a role config under
`<edge_home>/agents/<slug>.json`). The addressable, running unit is the session:
it mints its own key, publishes presence, and coordinates in channels. Agents
self-organize — the value is shared awareness that lets the left hand know what
the right hand is doing, not a durable per-agent identity.

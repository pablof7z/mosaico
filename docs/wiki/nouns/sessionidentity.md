---
type: noun-entry
slug: sessionidentity
name: "SessionIdentity"
origin: extracted
source_refs:
  - transcript:2546-2548
---

# SessionIdentity

A lean read-side struct (pubkey, agent slug, session id, legacy code) replacing
the old AgentInstance. `display_slug()` returns `agent/session`, and
`agent_ref()` returns `AgentRef(pubkey, agent/session)`. Used for routing,
rendering, and member display.

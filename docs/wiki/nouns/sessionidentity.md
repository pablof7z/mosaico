---
type: noun-entry
slug: sessionidentity
name: "SessionIdentity"
origin: extracted
source_refs:
  - transcript:2546-2548
---

# SessionIdentity

A lean read-side struct (pubkey, slug, codename) replacing the old AgentInstance. display_slug() returns the codename; agent_ref() returns AgentRef(pubkey, codename). Codename = friendly_short_code(session_id). Used for routing, rendering, and member display.

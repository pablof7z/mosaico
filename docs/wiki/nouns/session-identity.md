---
type: noun-entry
slug: session-identity
name: "session identity"
origin: extracted
source_refs:
  - transcript:2404-2407
  - transcript:2609-2626
---

# session identity

The keypair a session signs and is routed by, derived at start as
`derive(management_secret, session_id)`. It is the session's whole identity —
there is no separate base key or ordinal. Because it is derived from the session
id, it is stable across resume and re-derivable from the machine's management key
without being stored.

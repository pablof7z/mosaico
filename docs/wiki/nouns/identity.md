---
type: noun-entry
slug: identity
name: "Identity"
origin: extracted
source_refs:
  - transcript:102-104
---

# Identity

A session's keypair, derived from the machine's management key as
`derive(management_secret, session_id)`. Identity is per session and per machine:
the only stored secret is the machine's management key, and every session key is
re-derivable from it plus the session id, so identities are recoverable without
storing any nsec.

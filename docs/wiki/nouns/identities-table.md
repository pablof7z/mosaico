---
type: noun-entry
slug: identities-table
name: "identities (table)"
origin: extracted
source_refs:
  - transcript:696-701
---

# identities (table)

Maps each session's derived pubkey to its owning session and a resume binding.
Every session's key is derived from the machine's management key as
`derive(management_secret, session_id)`; this table records those per-session
pubkeys so the daemon can bound its `#p` subscription (the set of pubkeys it
listens for) and resume the right session when a mention arrives for an offline
session.

---
type: noun-entry
slug: identities-table
name: "identities (table)"
origin: extracted
source_refs:
  - transcript:696-701
---

# identities (table)

Derived signing keys the daemon publishes as. `(derivation root pubkey, ordinal)` plus per-session pubkeys map to their owning agent/session and a resume binding. Bounds the #p subscription (the set of pubkeys the daemon listens for) and resumes the right session when a mention arrives for an offline ordinal identity. Runtime ordinals start at 1.

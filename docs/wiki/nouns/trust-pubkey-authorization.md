---
type: noun-entry
slug: trust-pubkey-authorization
name: "trust (pubkey authorization)"
origin: extracted
source_refs:
  - transcript:1069-1070
  - transcript:1123-1123
  - transcript:881-881
---

# trust (pubkey authorization)

Pubkey trust is determined exclusively and always by NIP-29 channel membership. The mgmt backend key adds a session pubkey as a member to the channel they need, and removes them when the session expires. No cryptographic derivation is verified by peers — authorization runs through published kind:0 + NIP-29 membership.

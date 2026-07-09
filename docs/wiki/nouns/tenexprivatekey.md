---
type: noun-entry
slug: tenexprivatekey
name: "tenexPrivateKey"
origin: extracted
source_refs:
  - transcript:249-251
---

# tenexPrivateKey

The machine's management key (hex seckey) — the only secret stored on the machine.
Every session's keypair is derived from it as `derive(management_secret,
session_id)`, and the management key is what adds and removes session pubkeys in
NIP-29 channel membership.

---
type: noun-entry
slug: agent-identity
name: "agent identity"
origin: extracted
source_refs:
  - transcript:236-240
---

# agent identity

Identity is per session, not per agent. Each session mints its own Nostr keypair
at start as `derive(management_secret, session_id)`, where the machine's
management key (`tenexPrivateKey`) is the only stored secret. There is no durable
per-agent keypair; `<edge_home>/agents/<slug>.json` is role config (harness,
provider, model), not an identity. A session is addressed by its codename handle
`@<codename>@<host>`, and it is trusted in a channel only through NIP-29
membership.

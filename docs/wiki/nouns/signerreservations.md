---
type: noun-entry
slug: signerreservations
name: "SignerReservations"
origin: extracted
source_refs:
  - transcript:340-343
---

# SignerReservations

Superseded by per-session identity. There are no ordinal slots to reserve: every
session derives its own keypair from the machine's management key, so concurrent
sessions cannot collide on a shared identity and need no reservation map. See
[Tenex-Edge Agent Identity](../guides/tenex-edge-agent-identity.md).

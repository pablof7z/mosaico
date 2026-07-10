---
type: noun-entry
slug: per-session-pubkey-nsec-derivation
name: "per-session pubkey (nsec derivation)"
origin: extracted
source_refs:
  - transcript:1068-1071
  - transcript:1122-1124
---

# per-session pubkey (nsec derivation)

There is no base agent key. Every session mints its own keypair: nsec = derive(mgmt_secret, session_id). mgmt_secret is per-machine, so the same session_id on two machines yields different keys. Nothing is stored except the mgmt key + an append-only pubkey→session_id map for resume/routing.

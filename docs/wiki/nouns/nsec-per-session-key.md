---
type: noun-entry
slug: nsec-per-session-key
name: "nsec (per-session key)"
origin: extracted
source_refs:
  - transcript:1068-1071
  - transcript:1120-1124
---

# nsec (per-session key)

Derived per session as nsec = derive(mgmt_secret, session_id), where mgmt_secret is the per-machine management key. There is no base agent key; nothing is stored as a secret except the mgmt key plus an append-only pubkey→session_id map.

---
type: noun-entry
slug: per-session-key
name: "per-session key"
origin: extracted
source_refs:
  - transcript:1068-1068
  - transcript:1122-1122
---

# per-session key

There is no base agent key. Every session mints its own pubkey; nsec = derive(mgmt_secret, session_id). Mgmt_secret is per-machine so the same session_id on two machines produces different keys. Nothing stored as a secret except the mgmt key; any session is recoverable by re-derivation.

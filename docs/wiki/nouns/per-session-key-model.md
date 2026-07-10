---
type: noun-entry
slug: per-session-key-model
name: "per-session key model"
origin: extracted
source_refs:
  - transcript:1068-1068
  - transcript:1122-1122
---

# per-session key model

There is no base agent key; all keys are created at session start. nsec = derive(mgmt_secret, session_id), where mgmt_secret is per-machine, so the same session_id on two machines yields different keys automatically. Nothing is stored as a secret except the mgmt key plus an append-only pubkey→session_id map.

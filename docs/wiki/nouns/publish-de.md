---
type: noun-entry
slug: publish-de
name: "publish_de"
origin: extracted
source_refs:
  - transcript:490-504
---

# publish_de

Historical runtime closure that published `DomainEvent`s with the wrong signing
keys. Current runtime publishing signs each event with the publishing session's
own derived key (`derive(management_secret, session_id)`), carried through
`EngineParams`.

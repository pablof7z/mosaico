---
type: noun-entry
slug: publish-de
name: "publish_de"
origin: extracted
source_refs:
  - transcript:490-504
---

# publish_de

Historical runtime closure that published `DomainEvent`s with the wrong signing keys. The bug was that it ignored the selected agent instance and signed all sessions with the local derivation-root key. Current runtime publishing signs with the selected ordinal instance key carried through `EngineParams`.

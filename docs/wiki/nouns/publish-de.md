---
type: noun-entry
slug: publish-de
name: "publish_de"
origin: extracted
source_refs:
  - transcript:490-504
---

# publish_de

A closure in runtime.rs that captures provider and p.keys (the base agent keypair), then publishes a DomainEvent signed with those keys. It was hardcoded to always sign with base keys regardless of ordinal, causing the ordinal kind:0-clobbering bug.

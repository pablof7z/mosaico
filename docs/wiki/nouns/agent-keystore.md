---
type: noun-entry
slug: agent-keystore
name: "agent role config"
origin: extracted
source_refs:
  - transcript:656-656
  - transcript:808-815
---

# agent role config

The set of role configs on this machine, stored under
`<edge_home>/agents/<slug>.json`. Each file describes a role (harness, provider,
model) that can be launched locally — it holds no identity key. Session keys are
derived per session from the machine's management key, and channel membership is
governed separately by the NIP-29 group member list. (Formerly framed as an
"agent keystore" holding durable per-agent private keys; there are no such keys.)

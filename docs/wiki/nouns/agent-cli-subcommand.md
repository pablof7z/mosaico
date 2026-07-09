---
type: noun-entry
slug: agent-cli-subcommand
name: "agent (CLI subcommand)"
origin: extracted
source_refs:
  - transcript:656-657
---

# agent (CLI subcommand)

Manages the local role configs on THIS machine under
`<edge_home>/agents/<slug>.json` — the harness/provider/model definitions a
session can be launched with. These files hold no identity key; session keys are
derived per session from the machine's management key. Channel membership is
governed separately by the NIP-29 group's member list.

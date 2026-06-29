---
type: noun-entry
slug: profile-domain-event
name: "Profile (domain event)"
origin: extracted
source_refs:
  - transcript:169-183
  - transcript:430-464
---

# Profile (domain event)

The agent's published identity card: resolves pubkey to slug, tells a peer which machine the agent lives on, and declares the human owner(s) it belongs to (p-tagged). Encoded as kind:0 with content {"name": agent.slug}, a ["host", host] tag, p-tags for owners, and a ["backend"] tag when is_backend is true.

---
type: noun-entry
slug: root-channel
name: "root channel"
origin: extracted
source_refs:
  - transcript:384-384
  - transcript:410-410
---

# root channel

A channel with parent == '' — the top-level ancestor. A root channel uses its slug as both channel_h and name. Found by walking parent links up via channel_project_root (capped at MAX_CHANNEL_PARENT_DEPTH = 16).

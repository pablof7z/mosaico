---
type: noun-entry
slug: route-scope
name: "route_scope"
origin: extracted
source_refs:
  - transcript:258-274
---

# route_scope

The NIP-29 group id this session currently routes under — its channel when set, else its per-session room (`project`). All fabric publishing (chat/mentions/proposals), local chat routing, `who`/statusline scoping, and turn-context deltas key on this so `channels switch` actually moves the session to a different room without restarting. `project` alone is stale the moment `channel` is set.

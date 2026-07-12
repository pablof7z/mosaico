---
type: noun-entry
slug: daemon-inhibit
name: "daemon.inhibit"
origin: extracted
source_refs:
  - transcript:397-401
  - transcript:252-264
---

# daemon.inhibit

A sentinel file ($TENEX_EDGE_HOME/daemon.inhibit) whose presence tells hook-path daemon calls to fail open (return Ok(Null)) rather than spawning or contacting the daemon; created by `tenex-edge daemon stop`, cleared by non-hook commands.

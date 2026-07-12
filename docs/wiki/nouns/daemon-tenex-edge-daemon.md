---
type: noun-entry
slug: daemon-tenex-edge-daemon
name: "daemon (tenex-edge daemon)"
origin: extracted
source_refs:
  - transcript:149-149
  - transcript:358-377
---

# daemon (tenex-edge daemon)

A separate, long-running per-machine process that holds all session, channel, and roster state; auto-spawns when an MCP process connects. Spawned detached via setsid/process_group(0), stdio to daemon.log, survives parent exiting.

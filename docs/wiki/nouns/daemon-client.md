---
type: noun-entry
slug: daemon-client
name: "daemon client"
origin: extracted
source_refs:
  - transcript:140-148
---

# daemon client

A thin client that connects to the per-machine daemon, spawning it if absent; on connect it tries the UDS, acquires a startup flock if no answer, re-checks for racers, reclaims stale sockets, and spawns a detached daemon.

---
type: noun-entry
slug: inhibit-flag
name: "inhibit flag"
origin: extracted
source_refs:
  - transcript:252-253
  - transcript:308-312
---

# inhibit flag

The `tenex-edge daemon stop` mechanism to prevent hooks from respawning a daemon the user explicitly killed; when set (stop-inhibit file exists), hook-path daemon calls return Ok(Null) so hooks fail open rather than spawning.

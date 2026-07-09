---
type: noun-entry
slug: session-codename
name: "session_codename"
origin: extracted
source_refs:
  - transcript:1937-1947
  - transcript:1575-1587
---

# session_codename

The primary human-friendly handle for a session, derived deterministically from
its session id by `friendly_short_code(session_id)` as a `word-word-NNN` code.
Qualified with the host, `@<codename>@<hostname>` is the session's kind:0 profile
name and the p-taggable mention target that peers use to address it. Because it is
derived from the session id, the same session id always yields the same codename,
so a resumed session keeps its handle.

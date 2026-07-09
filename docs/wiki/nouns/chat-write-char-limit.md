---
type: noun-entry
slug: chat-write-char-limit
name: "CHAT_WRITE_CHAR_LIMIT"
origin: extracted
source_refs:
  - transcript:1426-1426
  - transcript:1673-1675
  - transcript:1835-1836
---

# CHAT_WRITE_CHAR_LIMIT

The write-time character-based cap (600 characters) on messages published via
`tenex-edge channel send`; messages exceeding it error out unless `--long-message`
is passed. Replaces the prior word-count guard that incorrectly reused
CHAT_RENDER_WORD_LIMIT.

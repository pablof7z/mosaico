---
type: noun-entry
slug: inbox
name: "inbox"
origin: extracted
source_refs:
  - transcript:635-640
---

# inbox

The inbound routing ledger and local idempotency record. Direct-message rows are keyed by inbound event and target local session; they start `pending` and become `delivered` once injected into a live tmux pane. Orchestration uses the same ledger with synthetic per-target keys so each add target can complete or retry independently.

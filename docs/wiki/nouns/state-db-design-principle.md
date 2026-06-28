---
type: noun-entry
slug: state-db-design-principle
name: "state.db design principle"
origin: extracted
source_refs:
  - transcript:431-437
---

# state.db design principle

a read-through cache of relay state only; no tables or columns that deviate from relay-materialized state; every question about permissions/membership answered from materialized relay events, never local flags

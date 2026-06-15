---
title: the last distill timestamp must only be
slug: the-last-distill-timestamp-must-only-be
topic: general
summary: The `last_distill` timestamp is only updated on a successful distillation, not on failure
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-14
updated: 2026-06-15
verified: 2026-06-14
compiled-from: conversation
sources:
  - session:215d979a-a054-4e2b-b349-851e0d874d6d
---

# the last distill timestamp must only be

## `last_distill` Update Semantics

The `last_distill` timestamp is only updated on a successful distillation, not on failure. A failed distillation allows retry after another `turn_first` window elapses, tracked by a separate `last_distill_attempt` timestamp. The distillation call retries a few times if it times out.

<!-- citations: [^215d9-7] [^215d9-14] [^215d9-17] -->

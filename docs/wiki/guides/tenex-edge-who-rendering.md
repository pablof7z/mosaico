---
title: Tenex-Edge Who Rendering
slug: tenex-edge-who-rendering
topic: tenex-edge
summary: `tenex-edge who --all-projects` renders through the unified fabric-context pipeline (`build_view` â `render_view`/`render_human_view`), producing the same for
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-03
verified: 2026-07-03
compiled-from: conversation
sources:
  - session:7d6bf2fe-8dc9-4bd0-aeeb-de1827bf68d1
---

# Tenex-Edge Who Rendering

## Unified Rendering Pipeline

`tenex-edge who --all-projects` renders through the unified fabric-context pipeline (`build_view` → `render_view`/`render_human_view`), producing the same format as single-project `who`. Hook injection (turn-start and PostToolUse), single-project `who`, and `who --all-projects` all share this single rendering pipeline via `render_fabric_context`. The regression test `who_all_projects_uses_unified_fabric_render_not_old_table` asserts that the old table format is gone from `--all-projects` output.

The old `src/cli/who/render.rs` + `src/who_snapshot.rs` markdown-table renderer (`WhoSnapshot`/`render_who_*`) is a fallback-only path used when the daemon emits no `fabric`/`fabric_human` string at all (e.g. version skew), on both single-project and `--all-projects` branches.

<!-- citations: [^7d6bf-cad7a] [^7d6bf-5aab0] -->
## `who --all-projects` Block Layout

`render_fabric_all_projects`/`_human` in `fabric_context.rs` renders one project block per root channel. Within each block, per-member entries are listed as `role @agent/session - status` (for example, `developer @developer/peer-sess - working`), followed by the available-agent roster materialized from kind:30555 events for that root channel. Availability rows show the advertised role slug, host, and use criteria; they are the role types that can be added to the channel, not concrete members.

<!-- citations: [^7d6bf-bafea] [^7d6bf-db5d3] -->

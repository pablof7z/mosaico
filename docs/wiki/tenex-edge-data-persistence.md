---
title: Tenex-Edge Data Persistence
slug: tenex-edge-data-persistence
topic: tenex-edge
summary: Local state is stored in SQLite
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-08
updated: 2026-06-09
verified: 2026-06-08
compiled-from: conversation
sources:
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:72c1c649-6826-4219-a8d4-b507abc78310
  - session:ccdf5ab7-5155-4b5f-8be9-866a2608a8ee
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:240ffb86-8827-4741-932b-29fb1824c0c7
  - session:05b89548-666c-4e24-a2f5-8a1e92f0bf04
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
  - session:ab9998c4-6e65-410e-b298-122a2072171c
---

# Tenex-Edge Data Persistence

## Data Persistence

All data must be read from a unified local interface (e.g., SQLite); how the data is hydrated into that store should be completely irrelevant to the use of that data. The unified interface must serve: the list of projects and their metadata; the list of agents inside a project and their metadata; the list of threads inside a project; the messages inside a thread; the recipient of each thread or message. Local state is persisted in SQLite at ~/.tenex/edge/state.db. The unified store extends the existing state.db schema mapping entities to real tables (project_meta.about, profiles, peer_sessions, inbox, agent_status), with threads as the one genuinely new table. The channel server acts as a thin stream-consumer that never independently writes state.db, avoiding multi-writer corruption. WAL mode (PRAGMA journal_mode=WAL) with busy_timeout and synchronous=NORMAL is enabled as an immediate stopgap to reduce corruption risk while the daemon architecture is built. Project metadata is stored in a `project_meta` SQLite table with columns `(project TEXT PRIMARY KEY, about TEXT NOT NULL, updated_at INTEGER NOT NULL)`. A spike is needed early to determine how NMP handles embedding in N concurrent per-session processes, specifically whether LMDB requires separate paths per process or supports a shared mode. OpenCode stalls on startup when its SQLite database schema is missing a 'name' column that the current version expects. To recover from a broken database schema, the database file must be renamed with a .bak suffix so that the process recreates it fresh on startup. A .bak suffix on state.db indicates a previously-broken DB that was renamed per convention, not a deliberate clean backup; the process recreates the DB fresh on startup if missing. `add_message_recipient` must be idempotent for the NULL `target_session` case, since SQLite treats NULL as distinct in the primary key, which causes unbounded duplicate rows on relay echo or repeated catch-up. The `rpc_thread_meta` handler must not return bare JSON null for a missing thread — it must return an empty object so the client doesn't interpret it as 'neither ok nor error'. The legacy `inbox`/`route_mention_into` path stays authoritative and frozen as the reader during the refactor — the canonical read model is written but the risky inbox-over-messages reader swap is deliberately retained per the doc's escape hatch.

<!-- citations: [^f3a73-29] [^f3a73-49] [^72c1c-1] [^ccdf5-1] [^162f9-7] [^240ff-5] [^05b89-3] [^162f9-13] [^d208c-16] [^d208c-24] [^ab999-29] [^ab999-67] -->

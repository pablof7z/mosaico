---
title: Tenex-Edge Phased Build
slug: tenex-edge-phased-build
topic: tenex-edge
summary: Phase 0 through Phase 8 are executed as a hard sequential dependency chain, with each phase gated by a validation ladder and a commit before proceeding.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-10
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:ab9998c4-6e65-410e-b298-122a2072171c
---

# Tenex-Edge Phased Build

## Phased Execution Model

The fabric-architecture refactor is implemented across 9 sequential phases (0–8) in a git worktree at /Users/pablofernandez/src/tenex-edge-fabric on branch 'fabric-architecture', with background agents fanned out in parallel only where compilation units are genuinely independent (Phase 0 splits the integration-binary vs the lib crate); phases that concentrate in state.rs/server.rs run single-agent sequentially. In-progress WIP must be committed to the main branch before creating the worktree branch from it, and a green test baseline (lib + mechanics + integration + e2e) must be established in the worktree before fanning out any phase work. Every phase join is validated by re-running the full test ladder (lib + mechanics + integration + e2e) before committing, rather than trusting agent reports; each completed phase is committed as a rollback point before proceeding to the next phase. Safe concurrency is limited to phases with genuinely independent file targets (Phase 0 and Phases 3–5 can fan out; single-file phases like Phase 1 cannot). The refactor adds zero net clippy lints and zero rustc warnings relative to the master baseline; intrinsic too_many_arguments lints on new accessors are suppressed.

<!-- citations: [^ab999-1] [^ab999-2] [^ab999-6] [^ab999-33] [^ab999-49] [^ab999-59] [^ab999-79] -->
## Per-Phase Parallelism

Phase 0 spawns exactly two parallel build units: one for the integration test binary (tests/daemon_integration.rs) and one for the lib crate (state.rs, runtime.rs, cli.rs test modules), adding 20 freeze_* regression tests pinning routing, dedup, ACL, context-from-store, and idempotency behaviors across disjoint compilation units before any refactoring touches them. Phases 1–4 are confined to single-agent work: Phase 1 introduces the canonical read-model schema (8 tables) with id generation (gen_id), accessors, and idempotent backfill (confined to state.rs), using additive migrations (a SCHEMA const with CREATE TABLE IF NOT EXISTS plus idempotent ALTERs, run in open()); Phase 2 splits Store into StoreReader/StoreWriter, routing every reader through read-model methods, while drain_inbox stays in turn-start as a delivery write and relay stays primary for project-list (internals may still bridge to legacy tables behind a TODO); Phase 3 extracts Nostr delivery (RawEnvelope, Scope, NostrDelivery, Kind1WireCodec) into src/fabric/*, moving filters out of the codec while keeping the existing filters-cover-all-kinds-and-mentions test untouched as the equivalence oracle; Phase 4 is a pure relocation where handle_incoming becomes a thin dispatch to fabric::materialize, with the doc's ACL membership-gating/quarantine explicitly deferred (not wired into the live path) to preserve frozen behavior and avoid breaking the integration freeze tests.

<!-- citations: [^ab999-3] [^ab999-7] [^ab999-34] [^ab999-50] -->
## Phases 5–8

Phase 5 introduces Kind1Nip29Provider bundling delivery/codec/materializer/lifecycle behind one provider, rewiring the daemon's four entry points (spawn_demux, rpc_session_start, ensure_subscription, fetch_mentions_into_inbox) while preserving the single-writer invariant via a shared Arc<Mutex<Store>>. Phase 6 introduces SendIntent/provider.send/sync_state and dual-writes canonical messages/message_recipients for both outbound and inbound, while the legacy inbox/route_mention_into path stays authoritative and frozen. Phase 7 adds thread/message reads (list_threads/messages/thread_meta as RPC+CLI) and provider-side reply threading via SendIntent.thread_id + NIP-10 root e-tag, deliberately leaving the Mention/domain type unchanged to preserve frozen mention tests. Phase 8 removes Codec::filters and SubScope from the seam, moves group builders into fabric/nip29/lifecycle.rs, wires backfill at daemon startup, moves rpc_project_list's inline 39000 fetch/parse into the provider, and documents the dual-written legacy tables as deliberately retained storage rather than swapping inbox-over-messages readers.

<!-- citations: [^ab999-8] [^ab999-35] [^ab999-51] -->
## Validation & Edge Cases

Startup backfill on a populated database runs correctly, carrying over project_origins, membership (with source=nip29-39002 and roles preserved), and about fields, with zero spurious rows on an empty db. The final verification is 120 lib + 4 mechanics + 14 integration + 1 e2e, 0 rustc warnings, 0 net-new clippy lints. The 39000/39002 inbound materialization is frozen only at the store-primitive level because the nak harness cannot emit relay-authored group events; the relay→materializer dispatch (rewritten in Phase 4) is covered by moved unit tests and the e2e group-create test. The --thread reply is verified at the component level (tag-encode + inbound grouping), not as one live send→receive→group round-trip within the automated test suite (though the live e2e run did exercise it). Live e2e verification is done with real claude, codex, and opencode agents running through the refactored daemon via an isolated home (TENEX_EDGE_HOME) and local relay, not mocks. The live e2e test with these agents found and fixed three bugs that unit tests missed: (A) add_message_recipient was not idempotent for NULL target_session (9 duplicate rows), (B) threads --project printed a truncated thread id unusable with --thread, (C) rpc_thread_meta returned bare JSON null for a missing thread causing 'neither ok nor error'. Live e2e testing with real opencode agent verified all three host adapters (claude-code hooks, codex te-hook.py, opencode TS plugin) carry real conversations through the refactored daemon.

<!-- citations: [^ab999-9] [^ab999-18] [^ab999-52] [^ab999-60] [^ab999-80] -->

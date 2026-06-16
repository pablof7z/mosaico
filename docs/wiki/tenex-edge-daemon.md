---
title: Tenex-Edge Daemon
slug: tenex-edge-daemon
topic: tenex-edge
summary: The process model is per-session (not a shared daemon)
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-16
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:ses_154369c0affeV2hnmjs7iYVX04
  - session:05b89548-666c-4e24-a2f5-8a1e92f0bf04
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:rollout-2026-06-09T15-01-20-019eac42-32f0-7ff0-bda2-da2de3b78ed7
  - session:1562957b-67e8-4ac1-a48b-84e8ec1696bb
  - session:rollout-2026-06-14T13-19-49-019ec5a5-1119-76f0-a7e3-36bc985a31bd
  - session:412e32c5-05f9-4e2a-86c6-e1c21e464553
  - session:1b868736-ed6b-4f88-84d9-26bb320accfd
---

# Tenex-Edge Daemon

## Architecture

The process model is per-session (not a shared daemon). (Previously: the direction was a single-machine single-writer daemon that solely owns state.db, with all CLI calls and session engines becoming thin IPC clients over a Unix domain socket — the multi-writer design where N per-session processes write to a single state.db was a confirmed failure mode that recurs without architectural changes; that daemon spawned on first tenex-edge invocation if absent (double-fork/setsid), bound to a Unix domain socket at $TENEX_EDGE_HOME/daemon.sock, used flock on daemon.lock for race-safe startup with stale-socket reclaim, served as the long-lived relay subscriber collapsing per-session relay connections, and idle-exited when no sessions were alive.) The persistence architecture must be fixed before it bites mid-session again; this decision shapes what the Claude Code plugin bootstraps (a per-session engine). The architecture extends state.db rather than replacing it; threads is the one genuinely new table. The build uses Rust and the nostr-sdk crate (not NMP); NMP was evaluated but found to be a full app kernel unsuitable for a headless CLI daemon — transport lives behind the codec seam so an NMP-backed transport remains a clean future swap-in. Debug-vs-release binaries and differing TENEX_EDGE_HOME values can evade the flock/socket lock and spawn parallel daemons; this robustness gap should be hardened. When multiple machines run the daemon and both see a mention for an agent, an ownership signal (from owned_groups or agent config) is needed so that only the machine that owns spawning for that agent actually spawns, preventing duplicate spawns across daemons. The daemon protocol version is bumped on response-shape changes to force stale daemons to respawn with the new snapshot shape. Deserialization of daemon responses is backward-compatible so that encountering a stale daemon response without the new fields does not crash the client. The isolated live e2e environment uses a separate TENEX_EDGE_HOME, a local nak relay, and PATH/TENEX_EDGE_* pointing at the worktree binary so it does not disturb the production ~/.tenex/edge daemon. Startup backfill on a populated database must work correctly — it migrates legacy data (projects, members, origins) into canonical tables without corruption. The production daemon cutover was completed: the refactored binary replaced the live daemon, the real state.db was migrated (40 projects, 40 origins, 15 members backfilled), and propose + threads are live verbs. The refactored daemon adds zero net clippy lints and zero rustc warnings compared to the master baseline.

<!-- citations: [^162f9-1] [^162f9-2] [^162f9-3] [^162f9-4] [^05b89-2] [^162f9-12] [^162f9-27] [^d208c-36] [^ab999-28] [^ab999-66] [^ab999-76] [^f3a73-112] [^rollo-18] [^15629-57] -->

## IPC Protocol

The IPC protocol is JSON-RPC over the Unix domain socket, with CLI verbs (who, inbox, send-message, turn-start/end) becoming RPCs. CLI calls daemon_call_async(method, params) via UDS JSON-RPC, dispatch() matches the method name, and a handler function processes it. spawn_session must atomically check-and-reserve a session_id under one lock before any .await point, so two concurrent session_start RPCs for the same session_id cannot both spawn a runtime. The daemon accept loop starts immediately after socket binding, before relay connections are established; relay connection and warmup happens in a background task while the accept loop is already serving RPCs. DaemonState holds transport and provider as Mutex<Option<Arc<...>>> so they can be populated once the background relay connection completes. A relay_ready Notify fires once when the relay connects, waking any handlers that called state.transport().await or state.provider().await. Relay-dependent operations (publish, subscribe, session start) wait via async transport()/provider() methods that wake as soon as set_relay() is called. The daemon opens one REQ subscription per hosted-agent × project combination. handle_incoming deduplicates events by event ID to prevent the same event from being processed multiple times due to multiple matching subscriptions; DaemonState contains a 512-slot ring buffer named seen_events for tracking seen event IDs, and duplicate events are short-circuited at the top of handle_incoming.

<!-- citations: [^56f9f-1] [^162f9-5] [^98f99-26] [^412e3-3] [^1b868-49] -->
## Database

WAL mode is enabled immediately (with busy_timeout and synchronous=NORMAL) as a stopgap while the daemon architecture is built, but the real fix is a single per-machine daemon that owns the database. state.db must always reside on a local disk (under ~/.tenex), satisfying WAL's same-machine requirement and the daemon's UDS assumption.

<!-- citations: [^162f9-6] [^162f9-17] -->
## Build & Install

The `just install` command builds the tenex-edge Rust binary in release mode and deploys it to `~/.local/bin/tenex-edge`. It then applies a macOS codesign fix (`xattr -cr` + `codesign --force --sign -`) to the deployed binary, which prevents a SIGKILL on fork/re-exec. After installing a new binary, the daemon must be restarted (pkill -f 'tenex-edge __daemon') so that subsequent hook calls spawn a fresh daemon from the newly installed binary, making new RPCs such as project_list, project_edit, and user_prompt available. After source commits and cleanup, the local release binary is reinstalled and the running daemon is restarted to pick up the changes. rustls CryptoProvider must be explicitly installed at process startup (ring provider) because rig-core's reqwest pulls both ring and aws-lc-rs, causing rustls 0.23 to panic when it can't auto-pick a default.

<!-- citations: [^ses_1-5] [^98f99-10] [^98f99-19] [^f3a73-113] [^rollo-37] -->

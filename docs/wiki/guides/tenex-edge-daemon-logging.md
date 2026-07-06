---
title: Tenex-Edge Daemon Logging
slug: tenex-edge-daemon-logging
topic: tenex-edge
summary: The daemon logs comprehensive operational events including routing to sessions, starting new agents (with reasons), ordinal creation (with reasons), subscriptio
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:47f3cac2-1ad9-461c-8ac0-3ea341d0e962
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
---

# Tenex-Edge Daemon Logging

## Overview

The daemon logs comprehensive operational events including routing to sessions, starting new agents (with reasons), ordinal creation (with reasons), subscriptions, and relay events. <!-- [^47f3c-2a5f9] -->


The slog per-session debug log filename uses the raw session_id for durable correlation. <!-- [^bd868-12ead] -->
## Log Level Strategy

The log level strategy is: ERROR for unrecoverable errors, WARN for recoverable-unexpected conditions, INFO for consequential single events, DEBUG for high-frequency or expected events. The default `RUST_LOG` filter for the daemon is `tenex_edge=info`. <!-- [^47f3c-e88ec] -->

## Log Format

For non-ANSI file output, the daemon log format is plain `HH:MM:SS LEVEL  message  key=value`.

In the daemon ANSI formatter, level badges are rendered as filled pills with inverted background colors: ` INF ` on cyan with black text, ` WRN ` on yellow, ` ERR ` on red, and ` DBG ` on dark gray. The timestamp is rendered in dimmed gray, the message text is bold white, field keys are dimmed, and field values are bright cyan. <!-- [^47f3c-23909] -->

## Startup & Shutdown

Daemon startup logs include socket bound, relay pool connected, warmup complete, spawn-on-mention coverage count, and shutdown. <!-- [^47f3c-ede20] -->

## Demux & Routing

Demux/routing logs include every incoming event at debug (kind+id+from), first-sight gate at info, duplicate delivery at debug, offline-agent-mention dispatch, orchestration dispatch, and the full spawn-on-mention decision tree. <!-- [^47f3c-6c9dd] -->

## Relay Events

Relay event lines (`[→relay]`/`[relay✗]`) are routed through `tracing::debug!` so they only appear on stdout at `RUST_LOG=tenex_edge=debug` and are always written to relay.log. <!-- [^47f3c-b622e] -->

## NIP-29 Role Decisions

NIP-29 role decision logs are emitted at `tracing::debug!` level with structured fields (group, target, role, reason). <!-- [^47f3c-7263e] -->

## Kind:30315 Status Heartbeats

The first-sight log for kind:30315 status heartbeat events is downgraded to `tracing::debug!` because they fire every ~30s per peer and are too noisy at info level. <!-- [^47f3c-fc46e] -->

## PTY Spawn Events

PTY spawn events (identity resolution failure, concurrent instance launch, pre-provisioning channel, provisioning timeout) are logged via `tracing::warn!` or `tracing::info!` with structured fields. <!-- [^47f3c-63b9b] -->

## Session Lifecycle

Session lifecycle logs include session_start hook received, stale session cancelled (with reason: same_pid/same_endpoint), re-assert when engine already running, session engine spawned with agent+channel+session, and session engine exited. <!-- [^47f3c-0b6d2] -->

## Ordinal Allocation

Ordinal allocation logs include agent key loaded (debug with slug and pubkey prefix), agent key created (info with slug, pubkey prefix, and path), ordinal slot allocated (info with session, agent, h, ordinal, and label), ordinal released, and preferred ordinal occupied (warn). <!-- [^47f3c-2d7ce] -->

## Subscriptions

Subscription logs include narrow REQ count when opening (debug), subscription timeouts (warn), and chat replay for spawn-on-mention (debug). <!-- [^47f3c-223bd] -->

## Reconcile

Reconcile logs include session count on restart, each dead session at warn with pid, and each revived session at info. <!-- [^47f3c-650c1] -->

## Orchestration

Orchestration logs include auth rejection (warn), parent mismatch (warn), per-target already-complete or in-flight skips (debug), identity mint (info), and spawn outcome (info/error). <!-- [^47f3c-4198f] -->

## Idle Watcher

The daemon idle watcher logs grace-period countdown start (info with grace_secs) and grace period elapsed exit (info). <!-- [^47f3c-e1cf4] -->

## Tracing Migration

All previous `TENEX_EDGE_DEBUG`-gated `eprintln!` calls in the daemon are replaced with structured `tracing` macros. <!-- [^47f3c-6f42f] -->

## CLI Debug Actions

CLI help text for DebugAction::HookTail.session reads 'Filter panes/events to a session id (or prefix).' <!-- [^bd868-06e51] -->

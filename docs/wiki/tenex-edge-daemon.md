---
title: Tenex-Edge Daemon
slug: tenex-edge-daemon
topic: tenex-edge
summary: tenex-edge uses a single machine-daemon that solely owns state.db, with all CLI calls and session engines acting as thin IPC clients over a Unix domain socket
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:ses_154369c0affeV2hnmjs7iYVX04
  - session:05b89548-666c-4e24-a2f5-8a1e92f0bf04
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
---

# Tenex-Edge Daemon

## Architecture

The multi-writer design where N per-session processes write to a single state.db is a confirmed failure mode that will recur without architectural changes. The persistence architecture must be fixed before it bites mid-session again; the current direction is a single-writer daemon. This decision shapes what the Claude Code plugin bootstraps (a single-writer daemon vs. spawning a per-session engine).
tenex-edge adopts a single-machine-daemon architecture that solely owns state.db, with all CLI calls and session engines becoming thin IPC clients over a Unix domain socket. The daemon is the long-lived relay subscriber, collapsing per-session relay connections so that wait-for-mention, future subscribe --json, and the channel adapter all stream from the daemon rather than touching SQLite directly.
The daemon spawns on first tenex-edge invocation if absent (double-fork/setsid), binds to a Unix domain socket at $TENEX_EDGE_HOME/daemon.sock, and uses flock on daemon.lock for race-safe startup with stale-socket reclaim.
The daemon idle-exits when no sessions are alive, tracked via session liveness/heartbeats.

<!-- citations: [^162f9-1] [^162f9-2] [^162f9-3] [^162f9-4] [^05b89-2] [^162f9-12] -->
## IPC Protocol

The IPC protocol is JSON-RPC or length-prefixed JSON lines over the Unix domain socket, with CLI verbs (who, inbox, send-message, turn-start/end) becoming RPCs. <!-- [^162f9-5] -->

## Database

WAL mode is enabled immediately (with busy_timeout and synchronous=NORMAL) as a stopgap while the daemon architecture is built, but the real fix is a single per-machine daemon that owns the database. state.db must always reside on a local disk (under ~/.tenex), satisfying WAL's same-machine requirement and the daemon's UDS assumption.

<!-- citations: [^162f9-6] [^162f9-17] -->
## Build & Install

The `just install` command builds the tenex-edge Rust binary in release mode and deploys it to `~/.local/bin/tenex-edge`. It then applies a macOS codesign fix (`xattr -cr` + `codesign --force --sign -`) to the deployed binary, which prevents a SIGKILL on fork/re-exec. After installing a new binary, the daemon must be restarted (pkill -f 'tenex-edge __daemon') so that subsequent hook calls spawn a fresh daemon from the newly installed binary, making new RPCs such as project_list, project_edit, and user_prompt available.

<!-- citations: [^ses_1-5] [^98f99-10] [^98f99-19] -->

---
type: episode-card
date: 2026-06-16
session: 412e32c5-05f9-4e2a-86c6-e1c21e464553
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/412e32c5-05f9-4e2a-86c6-e1c21e464553.jsonl
salience: root-cause
status: active
subjects:
  - daemon-startup
  - daemon-state
  - transport-connect
supersedes: []
related_claims: []
source_lines:
  - 1-1
  - 221-244
  - 689-705
  - 711-781
captured_at: 2026-06-16T11:51:44Z
---

# Episode: Daemon cold-start now lazy-loads relay; accept loop starts immediately

## Prior State

Daemon's run() bound the Unix socket first, then connected to relays (including NIP-42 AUTH warmup), and only then started the accept loop. On cold start, a client could connect to the socket immediately but blocked waiting for a welcome because no one was reading from it yet — the daemon was still doing Transport::connect (8+ seconds of relay connection). This made tenex-edge tmux appear to hang.

## Trigger

User reported slow tmux startup. Root-cause diagnosis: socket bind (server.rs:131) precedes accept loop (server.rs:208) by the full relay-connect + warmup duration. The client's spawn-if-absent succeeds on socket connect, retries try_call(), connects to the socket, sends hello, then blocks waiting for welcome — dead time the client silently pays.

## Decision

Restructured DaemonState so transport and provider are Mutex<Option<Arc<...>>> with a relay_ready: Notify signal. run() now starts the accept loop immediately after bind_socket (and constructing DaemonState with store-only fields). Relay connection happens in a background tokio::spawn; set_relay() atomically sets both fields and calls notify_waiters(). Store-only RPCs (who, tmux_*) render immediately from SQLite. Relay-dependent RPCs call state.transport().await / state.provider().await which wake on relay_ready.

## Consequences

- Cold-start latency for store-only operations drops from 8+ seconds to near-instant
- Relay-dependent operations (publish, subscribe, session start) still wait but are naturally blocked on relay anyway — no semantic change
- DaemonState field access pattern changed: transport and provider are now Option-gated with async getters rather than direct Arc fields; ~20 call sites updated
- provider_now() added for sync contexts that need graceful degradation when relay isn't ready yet
- reconcile_sessions and spawn_demux now run after relay connects inside the background task, not in the main run() sequence

## Open Tail

- No fallback/retry if relay connect fails in the background task — currently just logs an error; daemon stays alive but relay-dependent RPCs hang forever awaiting relay_ready

## Evidence

- transcript lines 1-1
- transcript lines 221-244
- transcript lines 689-705
- transcript lines 711-781


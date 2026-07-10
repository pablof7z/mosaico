---
type: episode-card
date: 2026-07-10
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: root-cause
status: active
subjects:
  - daemon-startup-critical-path
  - transport-connect-background
  - store-only-rpc-latency
supersedes: []
related_claims: []
source_lines:
  - 6077-6079
  - 6124-6144
  - 6207-6251
captured_at: 2026-07-10T00:10:32Z
---

# Episode: Daemon startup: relay connect moved off critical path to background spawn

## Prior State

The daemon awaited `client.connect()` on the startup critical path before spawning its accept loop. This meant relay-connect latency stalled even store-only RPCs (`who`, hosted sessions, chat/inbox reads) — directly contradicting the code's own comment that store-only RPCs 'serve instantly even when the relay is slow or unreachable.'

## Trigger

De-flake agent found `chat_commands_require_channel_when_session_joined_to_multiple_channels` failed ~1/46 with 'daemon did not answer handshakes within 30s.' Root cause traced to `src/daemon/server/lifecycle.rs`: the accept loop was spawned only after `Transport::connect_with_indexer(...).await` completed.

## Decision

`client.connect()` is now initiated in a `tokio::spawn` (background, non-awaited) inside `connect_with_indexer`. `warmup()` still awaits real connectivity + NIP-42 auth off the critical path, preserving the contract that store-only RPCs serve instantly regardless of relay state.

## Consequences

- Contract change: `Transport::connect` / `connect_with_indexer` no longer guarantees connected-on-return; callers that publish immediately must call `warmup()` first
- e2e_transport test required `warmup()` added after both reader and agent connect (matching the daemon's real usage pattern)
- All production callers verified safe: daemon lifecycle warmups before publishing; other callers use empty relay lists
- Landed in PR #330, merged to master

## Open Tail

- No production deploy performed — the live fleet still runs the old binary

## Evidence

- transcript lines 6077-6079
- transcript lines 6124-6144
- transcript lines 6207-6251

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-10-b70718e17221-cb4e41ba-2-daemon-startup-relay-connect-moved-off.json`](transcripts/2026-07-10-b70718e17221-cb4e41ba-2-daemon-startup-relay-connect-moved-off.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-10-b70718e17221-cb4e41ba-2-daemon-startup-relay-connect-moved-off.json`](transcripts/raw/2026-07-10-b70718e17221-cb4e41ba-2-daemon-startup-relay-connect-moved-off.json)

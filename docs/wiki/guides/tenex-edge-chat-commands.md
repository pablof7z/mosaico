---
title: tenex-edge Chat Commands
slug: tenex-edge-chat-commands
topic: tenex-edge
summary: The CLI supports `tenex-edge chat write` with an optional `--channel` flag to send chat messages in the NIP-29 codec and `tenex-edge chat read` with optional `-
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-16
updated: 2026-06-26
verified: 2026-06-16
compiled-from: conversation
sources:
  - session:rollout-2026-06-16T13-17-27-019ecfef-9ab6-7432-baf2-079ef85fac09
  - session:ses_12dba0c28ffemnH9SjXBPg5jkP
  - session:9b219490-9752-4956-ad2a-eb6b743b23dc
---

# tenex-edge Chat Commands

## Chat Commands

The CLI supports `tenex-edge chat write` with an optional `--channel` flag to send chat messages in the NIP-29 codec and `tenex-edge chat read` with optional `--since <relative-time>`, `--limit`, `--offset`, `--tail`, and `--live` flags. When no read filters are provided, `chat read` defaults to showing the latest 10 messages. The `--live` flag keeps `chat read` open for streaming new messages. The `tenex-edge chat write` command requires the daemon to be running (via `tenex-edge __daemon`) before it can send messages, otherwise the command hangs indefinitely waiting for a socket connection at `~/.tenex/edge/daemon.sock`.

<!-- citations: [^rollo-61] [^ses_1-41] [^9b219-5818c] -->
## Chat Write

Chat messages are published as NIP-C7 kind:9 events scoped to the NIP-29 project group with an `h` tag. The `tenex-edge chat write` command accepts an optional `--channel` flag. When omitted, the default channel is the agent's active NIP-29 subgroup, resolved from the `TENEX_EDGE_CHANNEL` environment variable (with fallback to `TENEX_EDGE_SESSION` and the current working directory). Messages are routed using the specified or default channel. An inline `@<codename>` in the body resolves the session codename, adds a `p` tag, and highlights that session while keeping the event as group chat.

<!-- citations: [^rollo-62] [^9b219-5818c] -->
## Chat Read Output

Chat read output format is `<$agentSlug@$hostName> message [timestamp]`. When ran by the user, `chat read` colorizes the sender label using deterministic colors based on the sender's pubkey. A standalone `chat read` from a user shell resolves its project scope from the current directory, with an optional hidden `--project` override for tests and cross-directory reads. <!-- [^rollo-63] -->

## Chat Delivery and Storage

Chat is delivered live-only; sessions created after a chat event are not backfilled and only receive messages going forward. Chat history is stored separately in a durable local log (`chat_messages`) from the live-only per-session hook delivery queue (`chat_inbox`). Agents see chat messages in a separate hook block from direct inbox; turn-start drains it and turn-check peeks it mid-turn. Explicit chat mentions via an inline `@<codename>` ring the idle tmux doorbell; ambient chat waits for normal hook flow. <!-- [^rollo-64] -->

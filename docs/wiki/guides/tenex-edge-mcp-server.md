---
title: Tenex-Edge MCP Server
slug: tenex-edge-mcp-server
topic: tenex-edge
summary: The MCP server (`tenex-edge mcp`) is a stateless stdio JSON-RPC loop spawned as a per-session subprocess by the harness
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-10
updated: 2026-07-10
verified: 2026-07-10
compiled-from: conversation
sources:
  - session:4d65680c-ded1-47cd-a59a-4966eebe8eda
---

# Tenex-Edge MCP Server

## Architecture

The MCP server (`tenex-edge mcp`) is a stateless stdio JSON-RPC loop spawned as a per-session subprocess by the harness. It forwards all tool calls over a Unix domain socket to a long-running daemon.

The daemon is a separate, long-running per-machine process that holds all session, channel, and roster state. It auto-spawns when an MCP process connects. <!-- [^4d656-d7fb2] -->

## MCP Tools

The MCP server exposes the following tools: `who`, `channels_list`, `chat_read`, `chat_write`, `channels_create`, `channels_join`, `channels_leave`, and `channels_switch`. <!-- [^4d656-4bc1c] -->

## Caller Identity & Session Resolution

The `who` tool sends caller-identity fields — `pty_session`, `harness`/`watch_pid`, `agent`, `group`, and `cwd` — read from environment variables to resolve a live Session via `CallerAnchor` resolution.

When `who` runs with no agent context, the MCP server auto-creates an agent context for that session rather than degrading to an anonymous snapshot. The no-anchor fallback uses `clientInfo.name` from the MCP `initialize` handshake to auto-provision a session with the naming convention `<clientInfo.name>/<random-suffix>`. For example, a ChatGPT client auto-creates a context displaying as "chatgpt/echo123", and a Grok client as "grok/echo123". Agent association is based on session identifiers such as the `x-openai` header, at least for ChatGPT. <!-- [^4d656-f20a0] -->

---
type: noun-entry
slug: mcp-server-mcp-process
name: "MCP server / MCP process"
origin: extracted
source_refs:
  - transcript:12-17
---

# MCP server / MCP process

A thin RPC translator — a stateless stdio JSON-RPC subprocess spawned per-session by an agent harness; every tool call is forwarded to a separate long-running daemon, holding no persistent state itself.

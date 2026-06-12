---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: architecture
status: active
subjects:
  - tenex-edge
  - claude-code-hooks
  - integration-mechanism
supersedes: []
related_claims: []
source_lines:
  - 214-333
captured_at: 2026-06-12T08:28:24Z
---

# Episode: tenex-edge Claude Code integration is hooks, not MCP server

## Prior State

The channel server code describes itself as an MCP server (stdio transport, @modelcontextprotocol/sdk), and the assistant initially configured .mcp.json + settings.local.json on the remote machine treating it as an MCP integration

## Trigger

User corrected: 'what? where's the mcp server? what? tenex-edge is rust, it doesn't use typescript' — the actual integration uses settings.template.json hooks calling the Rust binary directly

## Decision

The real tenex-edge ↔ Claude Code integration mechanism is hooks (SessionStart, UserPromptSubmit, Stop, SessionEnd) in ~/.claude/settings.json that shell out to `tenex-edge hook --host claude-code --type <event>`, not an MCP server. The .mcp.json and settings.local.json were removed from the remote and replaced with merged hook entries in the global settings.json

## Consequences

- The channel/ directory in integrations/claude-code/ exists but is NOT the primary integration path for Claude Code
- Hook configuration must be merged with existing hooks (e.g., pc awareness/capture hooks) rather than overwriting
- The tenex-edge binary must be on PATH (~/.local/bin/tenex-edge symlink) for hook commands to resolve

## Open Tail

*(none)*

## Evidence

- transcript lines 214-333


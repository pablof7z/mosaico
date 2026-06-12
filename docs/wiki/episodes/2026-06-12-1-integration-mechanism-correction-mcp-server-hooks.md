---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: reversal
status: active
subjects:
  - tenex-edge-integration
  - claude-code-hooks
supersedes: []
related_claims: []
source_lines:
  - 214-333
captured_at: 2026-06-12T08:41:38Z
---

# Episode: Integration mechanism correction: MCP server → hooks

## Prior State

Assistant assumed tenex-edge integrates with Claude Code as an MCP server (stdio), creating .mcp.json and settings.local.json with MCP server config pointing to server.ts run via bun

## Trigger

User explicitly corrected: 'what? where's the mcp server? what? tenex-edge is rust, it doesn't use typescript' — the channel server is a separate optional piece; the primary integration is hooks calling the Rust binary directly

## Decision

Removed .mcp.json and settings.local.json from remote; instead configured tenex-edge as hooks (SessionStart, SessionEnd, UserPromptSubmit, Stop) in ~/.claude/settings.json, merging alongside existing pc hooks

## Consequences

- Correct integration model deployed on remote machine
- The .mcp.json was deleted from the repo on remote (shown in git status)
- Hooks call tenex-edge binary directly without TypeScript/bun dependency

## Open Tail

- The channel/server.ts MCP integration still exists in the repo but is not the primary integration path for Claude Code

## Evidence

- transcript lines 214-333


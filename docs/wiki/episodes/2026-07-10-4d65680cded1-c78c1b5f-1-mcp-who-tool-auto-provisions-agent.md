---
type: episode-card
date: 2026-07-10
session: 4d65680c-ded1-47cd-a59a-4966eebe8eda
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/4d65680c-ded1-47cd-a59a-4966eebe8eda.jsonl
salience: product
status: active
subjects:
  - mcp-who-tool
  - session-resolution
  - clientinfo-identity
supersedes: []
related_claims: []
source_lines:
  - 26-48
  - 50-50
  - 57-62
captured_at: 2026-07-10T11:53:52Z
---

# Episode: MCP `who` tool auto-provisions agent context instead of degrading anonymously

## Prior State

When the MCP server was started without agent context (no PTY session, harness, watch-pid, or agent/group env), the `who` tool silently fell into `caller_rec = None`, producing an anonymous session-less fabric snapshot with no self-row and no `(you)` marker. The MCP `initialize` handshake already received `clientInfo` (name/version) from the connecting client and logged it, but discarded it rather than threading it through to session resolution.

## Trigger

User directive (line 50): with no agent context, the tool should create an agent context for that session, ideally showing it as 'chatgpt/echo123' for chatgpt or 'grok/echo123' for grok, possibly associating via session-id x-openai header.

## Decision

Capture `clientInfo.name` at MCP `initialize` time, stash it against the connection/session in the daemon, and have `rpc_who`'s no-anchor fallback auto-provision a session using `<clientInfo.name>/<random-suffix>` (matching existing `agent/session-fragment` convention like `@codex/te-18c0e7f875d4bb90-3`) instead of collapsing to an anonymous empty-string snapshot.

## Consequences

- `who` transitions from a pure read to a side-effectful operation (creates a session record on first call when no anchor is present).
- Identity is by-assertion (`clientInfo.name` is self-reported by the client), not verified — acceptable for display/awareness, not for security-sensitive operations.
- Header-sniffing (`x-openai-...`) approach was rejected in favor of `clientInfo` capture because stdio transport (the default) has no HTTP headers; `clientInfo` works across both stdio and http transports.
- Stale/dead explicit anchors still hard-fail with Strict scope bail as before; only the fully-unset case changes behavior.

## Open Tail

- Persistence model for auto-provisioned sessions: per-MCP-connection (dies with process) vs. something stabler for reconnection.
- Need to examine existing 'create session' paths in the daemon to see if `rpc_who` can call into an established registration flow rather than inventing a new one.

## Evidence

- transcript lines 26-48
- transcript lines 50-50
- transcript lines 57-62

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-10-4d65680cded1-c78c1b5f-1-mcp-who-tool-auto-provisions-agent.json`](transcripts/2026-07-10-4d65680cded1-c78c1b5f-1-mcp-who-tool-auto-provisions-agent.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-10-4d65680cded1-c78c1b5f-1-mcp-who-tool-auto-provisions-agent.json`](transcripts/raw/2026-07-10-4d65680cded1-c78c1b5f-1-mcp-who-tool-auto-provisions-agent.json)

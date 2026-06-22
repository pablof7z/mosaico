---
title: tenex-edge Host Integration (MCP & Hooks)
slug: tenex-edge-host-integration-mcp-hooks
topic: tenex-edge
summary: MCP is the lowest-common-denominator substrate that every supported host speaks; Claude Code hooks are the premium tier adding blocking capability and lifecycle
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-07
updated: 2026-06-16
verified: 2026-06-07
compiled-from: conversation
sources:
  - session:8a3eb1b2-7bbf-4761-ad1a-411a0a1fa666
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:3da7f7d8-c5a3-4065-be64-3a3a73dbb1d6
  - session:956595fb-fa6a-45f8-869c-b53cae16124f
  - session:2cee1bc6-0f1a-4746-9de6-68ca1a7e2737
  - session:05b89548-666c-4e24-a2f5-8a1e92f0bf04
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:f9bdcf4c-c972-46ff-91b8-9e30785d3331
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:9ac666e5-b468-4af2-be5e-83e5c8f2d1d2
  - session:55a2eb41-5ff1-4eb3-bdb8-7a4728422be5
  - session:a0037729-ad51-460a-880d-0a9699f6ee41
  - session:9337d29e-ac62-417c-8e99-0cc22cbbfad3
  - session:rollout-2026-06-09T00-10-41-019ea912-c93f-7c90-a16d-d46484711d29
  - session:rollout-2026-06-12T11-18-49-019ebae9-8fa7-73f1-844d-bea23bfb0193
---

# tenex-edge Host Integration (MCP & Hooks)

## Host Integration Strategy

MCP is the lowest-common-denominator substrate that every supported host speaks; Claude Code hooks are the premium tier adding blocking capability and lifecycle events. Where a host lacks hooks, tenex-edge degrades gracefully to MCP-server-as-observer (advisory only, no blocking). Codex has real hooks (SessionStart, UserPromptSubmit, PostToolUse, Stop in config.toml), but they only fire in interactive mode—not in `codex exec`—so the tenex-codex launcher wrapper is the reliable integration point. OpenCode uses a TypeScript plugin for integration (experimental.chat.messages.transform for injection, tool.execute.after for observation, event for session lifecycle). For Claude Code, the integration (hooks + the `tenex-edge` skill + dispatcher) is packaged as a plugin, with the tenex-edge Rust binary remaining a separate install. The plugin's SessionStart hook gracefully degrades when the tenex-edge binary is absent from PATH: it checks for the binary's presence and either bootstraps, downloads, or prints install instructions before no-oping. pc is reduced to inject + capture only (its awareness hooks and session-start are removed from settings.json); tenex-edge drives awareness and session lifecycle.

All three harness hook configurations point directly to the source tree to avoid drift with deployed copies. Config templates reference the binary directly with no Python dependency; Codex uses a `"__BIN__"` placeholder, Claude Code uses `tenex-edge` assuming it is on PATH. Python integration scripts are deleted — there is no Python dependency anywhere in the integration.

The wait-for-mention mechanism is the portable floor for all harnesses; the channel adapter is the Claude-specific ceiling. Claude Code's channels (notifications/claude/channel), Codex's app-server+turn/start (JSON-RPC), and OpenCode's POST /session/{id}/prompt_async all provide idle-session wake primitives. The channel server must not independently own the engine lifecycle (making the engine itself the MCP server is rejected) because it would break Codex/OpenCode support and violate the host-agnostic boundary. <!-- [^162f9-3] -->

The Claude Code channel adapter (integrations/claude-code/channel/) is a thin Bun MCP server that runs a self-re-arming wait-for-mention loop, emitting <channel> events to wake idle sessions, and exposes a reply tool that shells send-message. It requires the --dangerously-load-development-channels server:tenex-edge launch flag (Claude Code v2.1.80+, Anthropic auth only, not Bedrock/Vertex). An idle Claude Code session is woken by a <channel source="tenex-edge"> event with no human prompt, and replies via the reply tool. <!-- [^162f9-4] -->

All diagnostics in the channel server go to stderr; child stdout is piped/consumed, never inherited, so the MCP stdout wire stays clean. The binary is resolved via TENEX_EDGE_BIN → ~/.local/bin/tenex-edge because the bare name is not on the spawned PATH. <!-- [^162f9-5] -->

Claude Code's installed settings include four tenex-edge hooks: `SessionStart` (`tenex-edge hook --host claude-code --type session-start`), `UserPromptSubmit` (tenex-edge first, then `pc inject`), `Stop` (tenex-edge first, then `pc capture --in 300`), and `SessionEnd` (tenex-edge first, then `pc capture`). The existing `pc` hooks are retained alongside tenex-edge hooks because they serve a separate purpose (proactive context). <!-- [^f9bdc-1] -->

Claude hooks find and watch the ancestor claude PID, matching Codex's existing behavior.

<!-- citations: [^162f9-3] [^162f9-4] [^162f9-5] [^f9bdc-1] [^3da7f-2] [^8a3eb-15] [^f3a73-14] [^2cee1-1] [^05b89-1] [^rollo-42] -->
## Hook-Driven Context Injection

Context-injection logic (inbox drain, peer roster, status changes) lives in the Rust binary, not in wrapper scripts. This `turn-start` command outputs the context the agent should see (inbox messages, peer presence/status changes since last update) directly, rather than leaving that assembly to wrapper scripts. The `turn_start` function is async and accepts a `--json` flag; when `--json` is set, it outputs `{"systemMessage": "..."}` for Codex, otherwise plain text for Claude Code. First turn emits the wait-for-mention hint plus the full peer roster; subsequent turns emit only deltas — inbox drains, new peers (`first_seen >= prev_turn_started_at`), and status changes (`updated_at >= prev_turn_started_at`). The `render_who_plain` function renders the peer roster without ANSI escape codes, using `●`/`○` dots and plain status text, suitable for context injection.

The `session-start` hook outputs JSON with a `systemMessage` field (e.g. `json.dumps({"systemMessage": msg})`) instead of plain text, because Codex parses all hook stdout as JSON. All Codex hook output types share the same base JSON schema with optional fields: `systemMessage` (string), `suppressOutput` (bool), `stopReason` (string), `hookSpecificOutput` (object).

The `UserPromptSubmit` hook creates a kind:1 OP (root event with no e-tag) signed by the userNsec stored in ~/.tenex/config.json, publishes it to the NIP-29 group via an h-tag with the project slug, and p-tags the agent pubkey that will process the message. It fails open — if userNsec is absent, the session is not found, or relay publish fails, it logs an error via eprintln and returns Ok(()) rather than propagating the error.

A `turn-check` (or `TurnCheck`) command provides a mid-run hook for PostToolUse that checks for incoming messages or status changes while an agent is working. `TurnCheck` is a pure-read command using `peek_inbox` (no writes to state.db) so PostToolUse hooks add zero new concurrent writers. Claude Code PostToolUse hooks must output context via `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"..."}}` with exit 0; plain stdout is ignored by Claude Code, so PostToolUse is only wired for Codex, not Claude Code. Claude Code's `UserPromptSubmit` stdout is plain text developer context injection with no JSON wrapping required.

<!-- citations: [^f3a73-15] [^3da7f-1] [^95659-6] [^2cee1-2] [^98f99-7] [^a0037-4] -->
## Host Definition Registry

The integration uses a data-driven `HostDef` registry so adding a new agent harness requires only one struct definition with no new dispatch code. The sole host-facing entry point for hook-driven lifecycle actions is `tenex-edge hook --host <name> --type <type>`, accepting types `session-start`, `session-end`, `user-prompt-submit`, `post-tool-use`, and `stop`. Each `HostDef` specifies: `name`, `agent_slug`, `session_id_fields` (list of possible JSON field names for session ID), `transcript_field` (optional), `output_format` (PlainText vs JsonSystemMessage), `pid_search` (optional process-tree walk), `generates_sid` (flag gating whether session-start generates and prints a new SID to stdout when none is supplied — specific to opencode; for Claude Code and Codex, an empty id remains a fail-open no-op to prevent spawning orphan sessions), and an explicit `pid` field in the JSON payload for programmatic hosts like opencode that know their own process (bypassing the ancestor `pid_search` heuristic). Unknown hook types fail open with an eprintln so future harness versions don't break old binaries.

The tenex-edge Codex integration uses native Codex hooks rather than a wrapper. Codex uses tenex-edge hooks configured in ~/.codex/config.toml for SessionStart, UserPromptSubmit, and Stop. The Codex config template uses direct binary invocation without an `__HOOK__` substitution step.

The opencode tenex-edge plugin is located at ~/.config/opencode/plugin/tenex-edge.ts and uses the unified 'tenex-edge hook' entry point, piping JSON on stdin for event types session-start (with {cwd, pid}), user-prompt-submit, and stop. The opencode integration injects the stdout from the `user-prompt-submit` hook directly at turn start, instead of rebuilding context blocks in TypeScript. It maps its repeated mid-turn `transform` calls to the `post-tool-use` hook (the non-destructive peek path) instead of shelling out to `tenex-edge inbox` and `tenex-edge who`. Opencode's mid-turn awareness is delta-gated via `turn_check`, matching the drain-once-at-turn-start / peek-mid-turn split used by Claude Code, instead of re-listing the full inbox and roster on every model invocation.

Integration docs (README.md, integrations/codex/README.md, integrations/codex/AGENTS.md, and docs/wiki/tenex-edge-host-adapter.md) are updated to describe the hook-based Codex integration rather than the wrapper. <!-- [^55a2e-2] --> <!-- [^55a2e-3] -->

<!-- citations: [^55a2e-2] [^55a2e-3] [^2cee1-3] [^f9bdc-2] [^9ac66-4] [^9337d-2] [^rollo-4] -->
## Peer Session Tracking

The `peer_sessions` table has a `first_seen` column populated only on INSERT (not on heartbeat conflict updates) to accurately track when a peer appeared. This enables delta-based turn output: subsequent turns can identify new peers via `first_seen >= prev_turn_started_at`. <!-- [^2cee1-4] -->

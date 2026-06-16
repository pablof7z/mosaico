---
title: OpenCode Configuration
slug: opencode-configuration
topic: tenex-edge
summary: The @opencode-ai/plugin dependency version must match the opencode binary version (1.16.2) in both ~/.config/opencode/package.json and ~/.opencode/package.json
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-08
updated: 2026-06-16
verified: 2026-06-08
compiled-from: conversation
sources:
  - session:96aedf14-df2c-425b-b548-0fa7d1c1ba63
  - session:956595fb-fa6a-45f8-869c-b53cae16124f
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:72c1c649-6826-4219-a8d4-b507abc78310
  - session:9337d29e-ac62-417c-8e99-0cc22cbbfad3
---

# OpenCode Configuration

## Dependency Version

The @opencode-ai/plugin dependency version must match the opencode binary version (1.16.2) in both ~/.config/opencode/package.json and ~/.opencode/package.json. The opencode binary lives at ~/.opencode/bin/opencode. Testing must also include the opencode agent adapter (TS plugin-based integration) alongside claude-code and codex. The opencode integration's run() and stripAnsi() helpers are removed as dead code. The integration does not capture the SHORT_CODE from the session-start response.

<!-- citations: [^9337d-6] [^9337d-7] [^96aed-6] [^96aed-7] [^95659-3] [^ab999-25] [^96aed-9] [^9337d-8] -->
## Session Hooks

OpenCode's `session.idle` hook must be verified to fire per-turn rather than mid-loop to prevent premature idle states during long turns. In headless `opencode run` mode, the plugin's fire-and-forget session-start races the single turn, so the session must be pre-registered deterministically via the hook. <!-- [^ab999-26] -->


The opencode integration acts as a dumb pipe passing hook stdout, not assembling its own context blocks. It does not build its own selfLine, call tenex-edge inbox, or call tenex-edge who. The self-identity line, inbox, chat, and presence content lives in exactly one place: the Rust turn.rs file (assemble_turn_start_context). The canonical self-identity injection for Claude Code and Codex comes from the Rust hook path, not from TypeScript. The opencode integration renders identically to Claude Code and Codex. <!-- [^9337d-9] -->

The opencode integration injects the stdout of runHook('user-prompt-submit') verbatim at turn start, providing the self-identity line, drained inbox, project chat, and peer roster from the shared Rust path. It injects the stdout of runHook('post-tool-use') verbatim for mid-turn model invocations, providing non-destructive inbox peeks and sibling deltas via the Rust turn_check path. No Rust changes are needed for this integration; the post-tool-use hook was already wired and emits plain stdout for opencode. The integration uses a drain-once/peek-mid-turn split for inbox awareness, rather than re-listing inbox and who on every model invocation. <!-- [^9337d-10] -->

The opencode integration's run() and stripAnsi() helpers are removed as dead code. The integration does not capture the SHORT_CODE from the session-start response. <!-- [^9337d-11] -->
## Stale Database Recovery

When opencode's local SQLite database has an outdated schema missing a required column, the database files (including WAL and shared memory) are backed up with a .bak suffix and removed, allowing opencode to recreate them with the correct schema on restart. This backup-and-remove process results in the loss of local conversation history. <!-- [^72c1c-2] -->

## Plugin Files

OpenCode plugin .ts files reside in ~/.config/opencode/plugin/ and are loaded from there on startup. The tenex-edge plugin file (tenex-edge.ts) is installed from ~/src/tenex-edge/integrations/opencode/. The proactive-context plugin file (proactive-context.ts) is installed from ~/src/proactive-context/integrations/opencode/. <!-- [^96aed-10] -->

---
title: Tenex-Edge Hook Subcommand
slug: tenex-edge-hook-subcommand
topic: tenex-edge
summary: The `hook` subcommand is the only host-facing entry point for harness integrations, dispatching to the same inner functions as the standalone verbs while adding
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-12
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:9ac666e5-b468-4af2-be5e-83e5c8f2d1d2
  - session:081ec521-c99b-42fb-9aa7-4a109519a62f
---

# Tenex-Edge Hook Subcommand

## Hook Subcommand Role

`tenex-edge hook --host <name> --type <hook-type>` is the sole host-facing entry point for session and turn lifecycle operations. It dispatches to internal private functions while adding stdin JSON parsing, session-id extraction, PID handling, and output-format selection. The `user-prompt-submit` hook type does more than the bare `turn-start` verb by also publishing the prompt as a kind:1 OP, making `hook` the real surface for that behavior. The opencode integration was migrated from calling bare verbs directly to piping a JSON payload to `tenex-edge hook` and reading stdout back; it passes an explicit `pid` field in its hook JSON payload rather than relying on ancestor pid_search.

The tenex-edge integration on the remote machine is configured via hooks in ~/.claude/settings.json calling the Rust binary directly, not via an MCP server. <!-- [^081ec-1] -->

The tenex-edge Rust binary is symlinked at ~/.local/bin/tenex-edge so hook commands can find it. <!-- [^081ec-2] -->

<!-- citations: [^9ac66-1] [^9ac66-8] -->
## Standalone Verb Status

The standalone verbs `session-start`, `session-end`, `turn-start`, `turn-check`, and `turn-end` are removed from the public CLI surface; they exist only as private functions called internally by `tenex-edge hook`. (Previously: they served as the callable core wrapped by hook and as manual/debug entry points.)

<!-- citations: [^9ac66-2] [^9ac66-9] -->
## Commands Outside Hook Scope

The commands `who`, `tail`, `doctor`, `project`, `acl`, and `send-message` are not superseded by `hook` because they are interactive queries, owner config, or agent-initiated actions. The `inbox`, `who`, `send-message`, and `wait-for-mention` commands remain on the CLI as manual/agent-facing commands. The `inbox` command specifically serves the opencode injection path and manual message-checking use; Claude Code and Codex drain via the hook path.

<!-- citations: [^9ac66-3] [^9ac66-11] -->
## Help Text Updates

The `--help` descriptions for `session-start`, `session-end`, `turn-start`, `turn-check`, and `turn-end` should be updated to reflect that they are private internal functions invoked by `hook`, removing any implication that harnesses or users call them directly as CLI commands.

<!-- citations: [^9ac66-4] [^9ac66-10] -->

## SID Generation and HostDef Flags

The `hook` session-start path generates and prints a new SID to stdout when the opencode HostDef is used with an empty session id. SID generation on empty session id is gated behind a `generates_sid` HostDef flag, restricted to opencode; for Claude Code and Codex, an empty id remains a fail-open no-op to prevent spawning orphan sessions. <!-- [^9ac66-12] -->

## Testing

The daemon integration smoke test drives the `hook` path via stdin and exercises the generate-and-print-SID branch. <!-- [^9ac66-13] -->

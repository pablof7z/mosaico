---
type: episode-card
date: 2026-06-16
session: 9337d29e-ac62-417c-8e99-0cc22cbbfad3
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9337d29e-ac62-417c-8e99-0cc22cbbfad3.jsonl
salience: architecture
status: active
subjects:
  - opencode-integration
  - turn-context-injection
  - hook-stdout-protocol
supersedes: []
related_claims: []
source_lines:
  - 1-524
captured_at: 2026-06-16T15:14:51Z
---

# Episode: opencode plugin becomes dumb pipe — Rust hooks are sole source of truth for turn context

## Prior State

The opencode TypeScript plugin duplicated context assembly that the Rust hook path already produced. It discarded hook stdout, rebuilt the self-identity line, shelled out to `tenex-edge inbox` and `tenex-edge who` separately in TS, and re-listed inbox+presence on every mid-turn model invocation (no drain-once/peek split). The self-identity line (slug + session short code) had to be maintained in two places: Rust `turn.rs` and TS `selfLine`.

## Trigger

User challenged why the opencode integration rebuilds context in TypeScript instead of consuming the hook's stdout, calling the duplication unnecessary and asking why opencode doesn't just read what tenex-edge hooks print.

## Decision

Refactored opencode plugin to inject hook stdout verbatim instead of rebuilding context in TS. Turn-start now injects stdout from `runHook("user-prompt-submit")`; mid-turn checkpoints inject stdout from `runHook("post-tool-use")` (the non-destructive `turn_check` path). Deleted: bespoke `selfLine`, `hinted` flag, `run()`, `stripAnsi()`, `SHORT_CODE` capture from session-start, and the `tenex-edge inbox`/`who` shell-outs.

## Consequences

- Rust `turn.rs` is now the single source of truth for all context injection (self-identity, inbox, chat, presence) across all three harnesses (Claude Code, Codex, opencode)
- opencode now gets the proper drain-once / peek-mid-turn split — inbox is drained once at turn start, subsequent checks are non-destructive rate-limited peeks via `turn_check`
- opencode agents will see the unified `tenex-edge inbox send --to ...` messaging guidance instead of the old opencode-specific `send-message` phrasing
- Net −25 lines; future context-format changes only need to touch Rust
- No Rust changes required — opencode's `post-tool-use` hook was already wired to emit plain stdout

## Open Tail

- A concurrent session-state rearchitecture (session 8c22fb) is touching hooks/runtime/server/who/codec — if it changes what user-prompt-submit/post-tool-use emit on stdout, the plugin will automatically track it (by design), but should be verified after that refactor lands

## Evidence

- transcript lines 1-524


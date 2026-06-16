---
type: episode-card
date: 2026-06-16
session: 9337d29e-ac62-417c-8e99-0cc22cbbfad3
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/9337d29e-ac62-417c-8e99-0cc22cbbfad3.jsonl
salience: architecture
status: active
subjects:
  - opencode-context-injection
  - turn-start-source-of-truth
  - hook-stdout-consumption
supersedes: []
related_claims: []
source_lines:
  - 1-210
  - 243-271
  - 375-447
  - 485-524
captured_at: 2026-06-16T11:55:44Z
---

# Episode: opencode plugin becomes dumb pipe — context injection unified to Rust hook stdout

## Prior State

The opencode TypeScript plugin duplicated the Rust hook's context assembly in TS: it discarded the hook's stdout and hand-built the self-identity line, inbox, and peer roster via separate `tenex-edge inbox` / `tenex-edge who` shell-outs. This meant every change to agent-visible context (self-line, inbox formatting, messaging guidance) had to be implemented in two places. It also meant opencode re-listed the full inbox+roster on every mid-turn model invocation instead of using the daemon's rate-limited delta path.

## Trigger

User noticed the session-identity injection was added in both the Rust path and the opencode TS path, and challenged why the integration always duplicates logic instead of consuming the hook's stdout. Investigation revealed `runHook` already captures stdout — opencode just threw it away. The historical reason (non-destructive mid-turn peeks) was already solved by the `turn_check`/`PostToolUse` split that Claude Code uses.

## Decision

Refactored the opencode plugin to inject hook stdout verbatim instead of rebuilding context: turn-start maps to `user-prompt-submit` (destructive drain, full context), mid-turn refresh maps to `post-tool-use` (non-destructive peek via `turn_check`). Deleted all bespoke TS context assembly: `selfLine`, `hinted` flag, `run()`, `stripAnsi()`, `SHORT_CODE`, and the `inbox`/`who` shell-outs.

## Consequences

- Agent-visible context (self-line, inbox, chat, presence) now has a single source of truth in Rust `turn.rs` for all three harnesses (Claude Code, Codex, opencode)
- opencode now gets the proper drain-once/peek-mid-turn split that Claude Code already had, instead of re-listing full inbox+roster on every model invocation
- opencode agents see unified messaging guidance (`tenex-edge inbox send --to ...`) instead of the old opencode-specific `send-message` phrasing
- Net −25 lines; any future changes to Rust hook output automatically propagate to opencode with no TS changes needed

## Open Tail

- A concurrent session-state rearchitecture is in flight touching hooks/runtime/server/who/codec — no conflict with this commit, but if that refactor changes what user-prompt-submit/post-tool-use emit on stdout, the opencode plugin will automatically track it (by design)

## Evidence

- transcript lines 1-210
- transcript lines 243-271
- transcript lines 375-447
- transcript lines 485-524


# Codex integration

Codex integrates through Codex lifecycle hooks. Each hook calls the
`tenex-edge` binary directly:

    tenex-edge hook --host codex --type <hook-type>

Install the hooks by copying `config.template.toml` into either:

- `~/.codex/config.toml` for all Codex projects
- `<repo>/.codex/config.toml` for one trusted project

`tenex-edge` must be on PATH. Codex requires non-managed hooks to be reviewed
and trusted in `/hooks`.

The hook mapping is:

- `SessionStart` → `tenex-edge hook --host codex --type session-start`
- `UserPromptSubmit` → `tenex-edge hook --host codex --type user-prompt-submit`
  (marks the turn "working", injects pending chat messages and `tenex-edge who` output)
- `PostToolUse` → `tenex-edge hook --host codex --type post-tool-use`
  (read-only chat peek mid-turn; does not drain)
- `Stop` → `tenex-edge hook --host codex --type stop`
  (marks the session idle when the turn finishes)

The adapter accepts Codex session identifiers under `session_id`, `sessionId`,
`conversation_id`, `conversationId`, `thread_id`, or `threadId`, then uses that
value consistently as the tenex-edge session id. Codex hook payloads may carry a
live `transcript_path` (a JSONL rollout file) handed at `turn-start` and re-read
as the turn progresses.

Codex hooks only fire in the interactive TUI, not in `codex exec`.

Codex does not currently document a `SessionEnd` hook. The binary detects the
Codex ancestor process via `pid_search` and passes it to `session-start
--watch-pid`; the liveness reaper stops presence when Codex exits.

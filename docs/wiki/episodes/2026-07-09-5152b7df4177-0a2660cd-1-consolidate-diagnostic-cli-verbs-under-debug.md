---
type: episode-card
date: 2026-07-09
session: 5152b7df-4177-40ec-a67e-5e2d6ff58ac3
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/5152b7df-4177-40ec-a67e-5e2d6ff58ac3.jsonl
salience: product
status: active
subjects:
  - cli-surface-reorganization
  - debug-subcommand
supersedes: []
related_claims: []
source_lines:
  - 1-5
  - 302-316
  - 366-367
  - 568-568
  - 615-615
  - 822-826
captured_at: 2026-07-09T17:53:09Z
---

# Episode: Consolidate diagnostic CLI verbs under debug subcommand

## Prior State

`tenex-edge explain`, `tenex-edge validate`, and `tenex-edge doctor` were top-level CLI subcommands. `doctor` was documented in the README as the user-facing 'if anything looks off' troubleshooting command.

## Trigger

User directive to move explain/validate into the debug subcommand. For doctor, user gave a triage rule: if it's only relay probing, remove it; if it surfaces useful internal state for debugging why tenex-edge is in a given state, move it under debug too.

## Decision

All three diagnostic verbs (`explain`, `validate`, `doctor`) are now subcommands of the existing hidden `debug` group: `tenex-edge debug {explain,validate,doctor}`. `doctor` was kept rather than removed after investigation confirmed it returns internal Trellis reconciler state (surface modes, oracle status, suppressed-publish counts) alongside the relay probe, satisfying the user's 'put it behind debug' criterion. Top-level `Cmd::Explain`, `Cmd::Validate`, and `Cmd::Doctor` enum variants and dispatch arms were removed.

## Consequences

- User-facing invocation path changed for all three commands; old top-level forms now error with 'unrecognized subcommand'
- validate --targets catalog example strings updated from `tenex-edge validate ...` to `tenex-edge debug validate ...`
- messaging.rs warning message updated from `tenex-edge doctor` to `tenex-edge debug doctor`
- README, wiki guide, trellis-adoption.md, trellis-mapping.md, and e2e/README.md all updated to reflect new invocation paths
- Internal probe event content in provider.rs (`tenex-edge doctor {t}`) left as-is — it is the wire payload, not the CLI invocation
- 206+3 lib tests pass; fmt and clippy clean

## Open Tail

*(none)*

## Evidence

- transcript lines 1-5
- transcript lines 302-316
- transcript lines 366-367
- transcript lines 568-568
- transcript lines 615-615
- transcript lines 822-826

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-5152b7df4177-0a2660cd-1-consolidate-diagnostic-cli-verbs-under-debug.json`](transcripts/2026-07-09-5152b7df4177-0a2660cd-1-consolidate-diagnostic-cli-verbs-under-debug.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-5152b7df4177-0a2660cd-1-consolidate-diagnostic-cli-verbs-under-debug.json`](transcripts/raw/2026-07-09-5152b7df4177-0a2660cd-1-consolidate-diagnostic-cli-verbs-under-debug.json)

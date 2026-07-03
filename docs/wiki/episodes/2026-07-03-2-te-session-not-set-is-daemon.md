---
type: episode-card
date: 2026-07-03
session: abce9e9f-8f3e-4561-9dd3-684afd59be80
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/abce9e9f-8f3e-4561-9dd3-684afd59be80.jsonl
salience: root-cause
status: active
subjects:
  - te-session
  - daemon-staleness
  - version-skew
supersedes: []
related_claims: []
source_lines:
  - 1-1
  - 1062-1074
  - 1096-1099
  - 1271-1272
captured_at: 2026-07-03T10:34:00Z
---

# Episode: @te_session not set is daemon/CLI version skew, not a repo bug

## Prior State

The `@te_session not set` error in the statusline was assumed to be a reproducible code bug in the launch/session-start path.

## Trigger

User reported the error appearing consistently and asked for a fix. Investigation in a fully isolated daemon + local NIP-29 relay (croissant) could not reproduce it — `@te_session` was correctly stamped every time.

## Decision

Concluded the `@te_session not set` symptom is caused by daemon staleness: the live production daemon (pid 7923) started at 12:56 PM but the installed CLI binary was rebuilt at 13:04 PM, with ~30 commits landing on master that day touching daemon RPC, session-start, and provider internals. The daemon's own log showed errors (`write actions are disabled`, `ChannelGate::Degraded`) consistent with running stale code. No code change was made for this issue.

## Consequences

- Restarting the production daemon is the prescribed remedy for the `@te_session not set` symptom, not a code fix.
- Future debugging of similar runtime issues should first rule out daemon/CLI version skew before investigating repo code.
- The assistant declined to restart the shared production daemon because other active agents depend on it.

## Open Tail

- Production daemon has not been restarted; user was asked whether to do so but no confirmation received in this session.

## Evidence

- transcript lines 1-1
- transcript lines 1062-1074
- transcript lines 1096-1099
- transcript lines 1271-1272

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-03-2-te-session-not-set-is-daemon.json`](transcripts/2026-07-03-2-te-session-not-set-is-daemon.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-03-2-te-session-not-set-is-daemon.json`](transcripts/raw/2026-07-03-2-te-session-not-set-is-daemon.json)

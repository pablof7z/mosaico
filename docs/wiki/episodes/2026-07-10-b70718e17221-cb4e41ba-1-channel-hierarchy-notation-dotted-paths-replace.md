---
type: episode-card
date: 2026-07-10
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: reversal
status: active
subjects:
  - channel-hierarchy-notation
  - project-concept-purge
  - dotted-path-cli-surface
supersedes:
  - 2026-07-09-b70718e17221-cb4e41ba-1-channel-hierarchy-redesign-dotted-paths-project
related_claims: []
source_lines:
  - 1-35
  - 139-139
  - 6329-6348
captured_at: 2026-07-10T00:10:32Z
---

# Episode: Channel hierarchy notation: dotted paths replace slash-separated paths; 'project' concept fully collapsed

## Prior State

Channel references used forward-slash hierarchy (e.g. `tenex-edge/planning`), explicitly documented as 'never dots' in the channels guide. Although a prior session (2026-07-03) had already unified projects and channels into one recursive node with 'project' as a workspace-binding attribute, the duality still leaked into CLI, daemon, and docs, and the slash notation remained canonical.

## Trigger

User directive (lines 1-35): 'no longer concept of projects wrt channel organization — it just happens to be nested groups.' User specified dotted-path notation for all channel operations (join, read, invite) e.g. `tenex-edge channel read channel1.epic-513.research`, and requested removal of `tenex-edge chat` in favor of `channel read`.

## Decision

Channel hierarchy is expressed exclusively as dotted paths (`project1.epic-513.research`). The 'project' noun is fully purged from the codebase and replaced by 'workspace.' CLI surface unified under `channel` subcommands. The prior slash-separated notation and 'never dots' guide rule are reversed.

## Consequences

- All channel CLI commands (join, read, invite) now use dotted-path channel names
- `tenex-edge chat` removed; replaced by `tenex-edge channel read`
- Member lists must expose session-level identity (not just agent) so agents can invite specific sessions by ID
- Wiki noun `workspace-replaces-project-concept.md` created; `project` wording purged from render output (regression-guarded by test asserting no 'project' in agent-facing fabric context)
- Prior slash-notation guide rule is now historical

## Open Tail

- Whether agent-facing member lists now show session IDs (user flagged this as a question at line 21)
- Full implementation status unclear due to transcript truncation; user initially requested only exploration and a proposal

## Evidence

- transcript lines 1-35
- transcript lines 139-139
- transcript lines 6329-6348

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-10-b70718e17221-cb4e41ba-1-channel-hierarchy-notation-dotted-paths-replace.json`](transcripts/2026-07-10-b70718e17221-cb4e41ba-1-channel-hierarchy-notation-dotted-paths-replace.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-10-b70718e17221-cb4e41ba-1-channel-hierarchy-notation-dotted-paths-replace.json`](transcripts/raw/2026-07-10-b70718e17221-cb4e41ba-1-channel-hierarchy-notation-dotted-paths-replace.json)

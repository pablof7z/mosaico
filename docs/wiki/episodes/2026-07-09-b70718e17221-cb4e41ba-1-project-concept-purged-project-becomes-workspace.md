---
type: episode-card
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: architecture
status: superseded
subjects:
  - project-channel-unification
  - workspace-binding
  - root-channel
  - vocabulary-purge
supersedes: []
related_claims: []
source_lines:
  - 1-35
  - 137-139
  - 4274-4323
  - 4577-4670
  - 4678-4693
captured_at: 2026-07-09T19:06:30Z
---

# Episode: Project concept purged — 'project' becomes 'workspace' binding + 'root channel'

## Prior State

Projects and channels were treated as distinct nouns — a project was a top-level NIP-29 group owning a workspace (git checkout + machine), while a channel was the same group with a parent set. The codebase was ~80% collapsed but the duality still leaked into daemon, CLI, hook output, RPC methods, and database tables. Human-facing context rendered `<project name="X">` wrappers; CLI used `--project`/`--all-projects` flags; the database had a `project_roots` table; RPC methods were named `project_add`, `project_members`, `project_list`, etc.

## Trigger

User directive (line 1-35): 'help me change how we express hierarchy to make things way easier for agents — no longer concept of projects wrt channel organization, it just happens to be nested groups.' The user wanted the project/channel duality eliminated entirely, with 'project' becoming an attribute rather than a separate concept. GitHub issue #201 already tracked this as an open architecture refactor.

## Decision

The word 'project' was fully purged from the codebase and replaced with two distinct concepts: (1) 'workspace' — the machine+path binding attribute a channel may carry, and (2) 'root channel' — the parent-empty top-of-tree channel. 184 files changed: `crate::project`→`crate::workspace`, `project_roots` table→`workspace_roots` (with row-preserving migration + `projects.json`→`workspaces.json` legacy fallback), RPC methods renamed (`project_add`→`channel_add_member`, `project_members`→`channel_members`, `project_list`→`root_channels`, `project_edit` deleted as orphaned), CLI flags `--project`/`--all-projects`→`--root`/`--all-roots`, hook wrapper `<project name>`→`<workspace name>`, statusline/who output now says `Root:`/`other root channels`. NIP-29 wire protocol was intentionally left untouched (no 'project' on the wire).

## Consequences

- Human-facing agent context has zero 'project' wording (grep-verified on merged master e94a6f3f)
- Database migration copies legacy `project_roots` rows into `workspace_roots` and drops old table; fresh/already-migrated DBs no-op
- RPC wire method names changed (wire skew accepted) — any external caller using old `project_*` methods breaks
- On-disk map `~/.tenex-edge/projects.json` renamed to `workspaces.json` with one-time legacy read-fallback
- Rebased cleanly onto concurrent PR #324 (agent-slug in fabric context) — both changes coexist; 891→901 unit tests + 46 e2e green on merged master
- Live fleet daemon NOT deployed — still running old binary emitting `<project>`; rollout requires state.db migration + fleet restart, deliberately deferred to user's explicit trigger
- Shared checkout local master stuck at superseded commit 66f40fee; needs `git fetch && git reset --hard origin/master` when no agent has WIP

## Open Tail

- Dotted-path channel notation (e.g. `channel1.epic-513.research`) proposed by user but not confirmed as implemented in this session — existing guide said 'never dots', user proposed reversing to dots
- Session visibility in member lists — user noted member list must show the session (not just agent) to enable session-targeted invites; status unclear from this session
- Live deployment of new daemon (migration on live state.db + fleet restart) remains user's explicit call
- MCP `channel_add` tool (#320) and reply-envelope design-doc rewrite (#322) remain as tracked open issues

## Evidence

- transcript lines 1-35
- transcript lines 137-139
- transcript lines 4274-4323
- transcript lines 4577-4670
- transcript lines 4678-4693

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-b70718e17221-cb4e41ba-1-project-concept-purged-project-becomes-workspace.json`](transcripts/2026-07-09-b70718e17221-cb4e41ba-1-project-concept-purged-project-becomes-workspace.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-1-project-concept-purged-project-becomes-workspace.json`](transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-1-project-concept-purged-project-becomes-workspace.json)

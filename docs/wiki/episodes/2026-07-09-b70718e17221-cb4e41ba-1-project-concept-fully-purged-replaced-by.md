---
type: episode-card
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: reversal
status: active
subjects:
  - project-purge
  - workspace-concept
  - root-channel
  - cli-surface
  - rpc-rename
  - table-migration
supersedes:
  - 2026-07-03-1-projects-and-channels-unified-into-one
related_claims: []
source_lines:
  - 1-35
  - 3895-3972
  - 4160-4187
  - 4274-4324
  - 4330-4443
captured_at: 2026-07-09T16:06:52Z
---

# Episode: "Project" concept fully purged — replaced by workspace + root channel

## Prior State

"Project" was a distinct top-level concept across the entire system: a CLI command (`project list/init/edit`), a module (`crate::project`), a database table (`project_roots`), RPC methods (`project_add`, `project_list`, `project_edit`, `project_members`, `project_remove`), and pervasive in human-facing rendering — the `<project name="X">` hook wrapper injected into agent context every turn, "Project:" in the statusline, and "project blocks/tabs" in `who` output. Issue #201 had already unified project/channel at the data model level (~80% collapsed), but the duality still leaked into CLI, RPC, and rendering.

## Trigger

User directed the channel/identity redesign to collapse hierarchy for agent ergonomics. After #305 removed only the `project` *command*, user corrected: "didn't we say we were abandoning projects wording? I still see many things with 'project'" — and explicitly chose full purge including internal names ("Everything incl. internals").

## Decision

"Project" fully eliminated at every layer. CLI: `project list/init/edit` rehomed as `channel list --roots` / `channel init` / `channel edit <root> --about`; `Cmd::Project` variant deleted. Human-facing: `<project>` → `<workspace>`, statusline "Project:" → "Root:", `--project`/`--all-projects` → `--root`/`--all-roots`. Internals: `crate::project` → `crate::workspace`, `project_roots` table → `workspace_roots` (with row-preserving migration), `projects.json` → `workspaces.json` (legacy fallback). RPCs: `project_add`→`channel_add_member`, `project_members`→`channel_members`, `project_list`→`root_channels`, `project_edit` deleted (orphaned). Two replacement concepts: "workspace" (machine+path binding) and "root channel" (parent-empty top-of-tree). NIP-29 wire tag strings untouched.

## Consequences

- 184 files changed; rendered agent context verified to contain zero "project" by grep
- Table migration required: project_roots → workspace_roots with ensure_renamed in schema.rs; on-disk projects.json → workspaces.json with one-time legacy read
- RPC wire method names changed (skew accepted) — all dispatch, daemon/blocking.rs, CLI callers, and integration tests updated
- channel edit on a root now requires an agent session (strict anchor resolution) — old project edit could run from bare project dir by slug; cwd-fallback not restored
- PR #325 (the purge) is CI-green but blocked from merging by concurrent PR #324 (zephyr's fabric_context agent-slug work overlapping ~13 files); coordination ordering: #324 first, #325 rebases and absorbs
- rpc_project_edit was already orphaned by #305 and then deleted in the purge

## Open Tail

- PR #325 not yet merged — gated by #324 CI-red (fmt failure on zephyr's side) and external coordination
- Live-fleet daemon deployment (identity-table migration on live state.db + session restart) deferred as user's explicit call

## Evidence

- transcript lines 1-35
- transcript lines 3895-3972
- transcript lines 4160-4187
- transcript lines 4274-4324
- transcript lines 4330-4443

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-b70718e17221-cb4e41ba-1-project-concept-fully-purged-replaced-by.json`](transcripts/2026-07-09-b70718e17221-cb4e41ba-1-project-concept-fully-purged-replaced-by.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-1-project-concept-fully-purged-replaced-by.json`](transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-1-project-concept-fully-purged-replaced-by.json)

---
type: episode-card
date: 2026-07-03
session: bdb6c341-4dd4-48e7-9764-e80242beb005
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/bdb6c341-4dd4-48e7-9764-e80242beb005.jsonl
salience: architecture
status: superseded
subjects:
  - project-channel-unification
  - workspace-binding
  - roster-inheritance
  - awareness-inheritance
supersedes: []
related_claims: []
source_lines:
  - 1-73
  - 75-105
captured_at: 2026-07-03T07:33:43Z
---

# Episode: Projects and channels unified into one recursive node — 'project' becomes a workspace-binding attribute

## Prior State

Projects and channels were treated as distinct nouns — a project was a top-level NIP-29 group owning a workspace (git checkout + machine), while a channel was the same group with a parent set. The codebase was already ~80% collapsed (one resolver, identity = (parent, name)), but the duality still leaked into daemon/CLI/hook branching and human-facing rendering.

## Trigger

User proposed completely removing the concept of 'projects' and making everything nested channels.

## Decision

Four locked design calls, all on one decision surface: (1) One node type — 'channel'. Project becomes an optional workspace-binding field (machine + path) on a node, explicitly NOT a subtype/enum. (2) Membership is per-node (you are a member only where explicitly added); awareness inherits downward (visibility/deltas for all descendants of channels you're in). @-mentions across channels work but cause no membership side-effects. (3) Workspace binding resolves by nearest ancestor — root binds, descendants inherit; child subdirectory binding shadows parent (monorepo sub-projects free later). (4) Relay contract unchanged — every channel stays a NIP-29 group with a parent hint; migration is purely local state and rendering.

## Consequences

- Roster duality eliminated: one membership model at every tree level, no 'project agents' vs 'channel members' split.
- Sub-projects and monorepo splits fall out naturally from nearest-ancestor workspace resolution without special-case code.
- Fabric snapshot becomes one tree renderer instead of three tiers (project→channels→subchannels) with different rules.
- Relay-facing tables (relay_*) left as relay-sourced projections — no big-bang rename, purely local refactor.
- Membership-vs-awareness semantics flagged as the one irreversible decision requiring human ratification before mechanical work.
- Deep nesting is a capability but not promoted — two levels (root + task rooms) covers real use.
- Published as GitHub issue #201 with labels refactor:architecture, needs-human-policy, risk:high.

## Open Tail

- Human ratification of decision #2 (per-node membership + downward-awareness inheritance) required before implementation begins.
- Call-site audit needed: every place daemon/CLI/hook branches on 'is this a project or a channel' must be replaced with 'does this node have a workspace binding' or 'is parent empty'.
- NIP-29 group cost at scale remains unaddressed — no story yet for when a group is worth minting vs. when a thread suffices.

## Evidence

- transcript lines 1-73
- transcript lines 75-105

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-03-1-projects-and-channels-unified-into-one.json`](transcripts/2026-07-03-1-projects-and-channels-unified-into-one.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-03-1-projects-and-channels-unified-into-one.json`](transcripts/raw/2026-07-03-1-projects-and-channels-unified-into-one.json)

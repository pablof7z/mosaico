---
type: episode-card
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: reversal
status: active
subjects:
  - channel-hierarchy
  - cli-command-surface
  - project-channel-unification
  - session-visible-members
supersedes:
  - 2026-07-09-b70718e17221-cb4e41ba-1-project-concept-purged-project-becomes-workspace
  - 2026-07-09-b70718e17221-cb4e41ba-2-durable-per-agent-identity-mechanism-retired
related_claims: []
source_lines:
  - 1-35
  - 5575-5576
captured_at: 2026-07-09T22:26:36Z
---

# Episode: Channel hierarchy redesign: dotted paths, project concept eliminated, CLI commands unified under `channel`

## Prior State

Channel hierarchy used forward-slash paths (e.g. `tenex-edge/planning`), the guide explicitly said 'never dots.' 'Projects' were a distinct top-level noun separate from channels. The CLI had a separate `tenex-edge chat` command for reading/sending. Channel member lists showed agent names but not the underlying session identity, preventing agents from inviting specific sessions.

## Trigger

User directive (lines 1–35): make hierarchy 'way easier for agents' — channels should be expressed as dotted hierarchical paths like `project1.epic-513.research`, reading/joining/inviting should all go through `tenex-edge channel` (removing `chat`), and the member list must show sessions so agents can invite precise sessions they observe working on related epics.

## Decision

Adopted dotted-path hierarchy (`channel1.epic-513.dev`); killed the `chat` and `project` CLI commands, rehoming read/join/invite/list/init/edit under `channel`; 'project' becomes a workspace-binding attribute on a channel, not a separate concept/table/code path (closes issue #201); channel member lists must render session identity; codename doctrine removed; mkdir -p auto-create semantics for channel paths; no depth cap on nesting; per-session-pubkey identity direction recommended (derive from base key, kills OrdinalSlot allocator).

## Consequences

- Agents can parse the full organizational hierarchy from the channel path alone, no out-of-band project lookup needed
- CLI command surface is simplified: one `channel` verb covers read/join/invite/create/list
- Session visibility in member lists is now a required contract for agent-to-agent invitation flows
- Issue #201 (collapse project into channel) is operationally closed — the duality is purged from schema, resolver, CLI, and docs
- Per-session-pubkey identity replaces OrdinalSlot allocator, changing the key-deriation architecture

## Open Tail

- Session identity rendering in member lists needs verification against the example XML format in the user's request
- Per-session-pubkey identity migration from OrdinalSlot allocator not yet implemented

## Evidence

- transcript lines 1-35
- transcript lines 5575-5576

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-b70718e17221-cb4e41ba-1-channel-hierarchy-redesign-dotted-paths-project.json`](transcripts/2026-07-09-b70718e17221-cb4e41ba-1-channel-hierarchy-redesign-dotted-paths-project.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-1-channel-hierarchy-redesign-dotted-paths-project.json`](transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-1-channel-hierarchy-redesign-dotted-paths-project.json)

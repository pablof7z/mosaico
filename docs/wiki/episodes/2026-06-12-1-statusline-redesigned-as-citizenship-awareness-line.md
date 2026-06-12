---
type: episode-card
date: 2026-06-12
session: e42f09d7-5fb0-438b-a356-216870390540
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/e42f09d7-5fb0-438b-a356-216870390540.jsonl
salience: product
status: active
subjects:
  - tenex-edge-statusline
  - claude-code-host-adapter
  - fabric-identity
supersedes: []
related_claims: []
source_lines:
  - 1-63
  - 65-66
  - 200-270
captured_at: 2026-06-12T11:40:35Z
---

# Episode: Statusline redesigned as citizenship awareness line

## Prior State

Statusline was conceived as a generic developer bar — model name, git branch, context budget, cargo check result — standard Claude Code patterns with no project-specific semantics.

## Trigger

User correction: 'that's not anchored enough on what this project is about... read the docs'; reading the project docs revealed the core thesis is citizenship (durable identity, NIP-29 group membership, presence heartbeat, fleet awareness), not generic dev metrics.

## Decision

The statusline is the 'floor product' (identity + awareness) rendered in the host. It should be a 'quiet citizenship line' — minimal when healthy (agent@host, session short code, heartbeat), loud only on four attention states: no group membership, inbox unread, sibling collision, ACL pending. This replaces the generic git/model bar concept entirely.

## Consequences

- The statusline must be a pure-read daemon verb (like peek_inbox) with no state.db writes, because Claude Code re-runs it constantly and concurrent transient writers are a documented failure mode
- Must fail open like other host adapters: daemon down → show degraded line or nothing, never block or error
- Membership warning shifts from injected turn context (which LLMs can ignore) to the statusline (which is always visible and self-corrects as cache refreshes)
- The canonical implementation path is a new `tenex-edge statusline --json` daemon verb reusing the read model behind `who` + `peek_inbox`

## Open Tail

- The `statusline` daemon verb has not yet been specified or implemented
- Which of the four attention states to wire first is undecided
- Integration with `.claude/settings.json` not yet done

## Evidence

- transcript lines 1-63
- transcript lines 65-66
- transcript lines 200-270


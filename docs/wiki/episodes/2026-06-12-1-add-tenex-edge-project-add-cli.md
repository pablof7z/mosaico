---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: product
status: active
subjects:
  - nip29-groups
  - tenex-edge-cli
  - project-add
supersedes: []
related_claims: []
source_lines:
  - 446-560
  - 1062-1098
captured_at: 2026-06-12T08:49:18Z
---

# Episode: Add `tenex-edge project add` CLI command for NIP-29 group membership

## Prior State

New agents were automatically added to NIP-29 groups at session-start via `group_put_user`, but there was no manual CLI command to add a member. If auto-add failed silently, the agent would be stuck with no recourse.

## Trigger

User asked whether new agents request access to NIP-29 groups and whether there's a way to add them from tenex-edge, then explicitly directed: "let's make it tenex-edge project add <project> <pubkey-or-npub-or-nip05>"

## Decision

Implemented `tenex-edge project add <project> <pubkey-or-npub-or-nip05>` as a new CLI subcommand and daemon RPC (`project_add`). Accepts hex pubkeys, npub/bech32, or NIP-05 identifiers (resolved via HTTP). Publishes a kind:9000 `put-user` event signed by the operator's `userNsec`, and caches the membership locally. Added `reqwest` dependency for NIP-05 resolution.

## Consequences

- Operators can now manually add agents to NIP-29 groups from any machine with relay admin access
- NIP-05 resolution introduces an HTTP network call in the CLI path, requiring `reqwest` as a new dependency
- The relay may reject the put-user if the operator key isn't an admin, surfacing a clear error

## Open Tail

*(none)*

## Evidence

- transcript lines 446-560
- transcript lines 1062-1098


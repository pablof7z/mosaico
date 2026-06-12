---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: product
status: active
subjects:
  - tenex-edge
  - nip29-groups
  - project-add-cli
supersedes: []
related_claims: []
source_lines:
  - 446-940
captured_at: 2026-06-12T08:28:24Z
---

# Episode: Add `tenex-edge project add` CLI command for manual group membership

## Prior State

New agents are only added to NIP-29 groups automatically on session-start via session.rs:207 calling group_put_user. There is no CLI command to manually add a member — `tenex-edge group` only has `list` and description-setting subcommands

## Trigger

User asked whether new agents request group access, and whether there's a way to add them from tenex-edge. On learning there's no manual path, user directed: 'let's make it tenex-edge project add <project> <pubkey-or-npub-or-nip05>'

## Decision

Add a `tenex-edge project add <project> <pubkey-or-npub-or-nip05>` CLI subcommand that resolves npub/nip05/hex to a pubkey and calls group_put_user (kind:9000) via the daemon RPC, signing with the operator's userNsec

## Consequences

- Needs a new daemon RPC handler (e.g. `project_add`) wired into demux.rs alongside existing project_list/project_edit
- Needs NIP-05 resolution (HTTPS GET to .well-known/nostr.json) — reqwest is already in the dependency tree
- PublicKey::parse from nostr-sdk handles hex/npub/NIP-21 already; NIP-05 requires separate network resolution
- The command must run through the daemon (async publish_signed_checked) rather than as a standalone CLI operation, since group state changes need relay acceptance

## Open Tail

- Implementation was started but not completed in this session
- NIP-05 resolution code still needs to be written

## Evidence

- transcript lines 446-940


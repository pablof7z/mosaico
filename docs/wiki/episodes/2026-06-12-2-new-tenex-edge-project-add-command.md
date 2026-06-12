---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: product
status: active
subjects:
  - tenex-edge-cli
  - nip29-groups
  - group-membership
supersedes: []
related_claims: []
source_lines:
  - 446-460
captured_at: 2026-06-12T08:41:38Z
---

# Episode: New `tenex-edge project add` command for NIP-29 group membership

## Prior State

No CLI command existed to add a pubkey to a NIP-29 group; group membership was only managed automatically at session-start via ensure_group_and_membership, which requires the daemon's operator key to already be a group admin

## Trigger

User asked whether new agents request NIP-29 group access and whether there was a way to add them from tenex-edge; directed: 'let's make it tenex-edge project add <project> <pubkey-or-npub-or-nip05>'

## Decision

Implemented `tenex-edge project add <project> <pubkey-or-npub-or-nip05>` CLI subcommand that accepts hex pubkeys, npub bech32, and NIP-05 identifiers. Resolves via PublicKey::parse for hex/npub and HTTP NIP-05 lookup for user@domain format. Calls daemon RPC `project_add` which publishes kind:9000 put-user signed by userNsec and caches the membership locally.

## Consequences

- New product surface for manual group membership management
- Added reqwest dependency (with rustls feature) for NIP-05 HTTP resolution
- New daemon RPC endpoint `project_add` wired in connection.rs
- NIP-05 resolution requires network access from the daemon

## Open Tail

*(none)*

## Evidence

- transcript lines 446-460


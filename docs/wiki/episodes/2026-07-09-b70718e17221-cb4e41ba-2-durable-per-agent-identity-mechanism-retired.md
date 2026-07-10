---
type: episode-card
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: architecture
status: superseded
subjects:
  - identity-model
  - per-session-keys
  - citizen-framing
  - durable-keypair-retirement
supersedes:
  - 2026-07-09-b70718e17221-cb4e41ba-2-durable-identity-mechanism-retired-in-docs
related_claims: []
source_lines:
  - 3983-4000
  - 4009-4036
captured_at: 2026-07-09T17:55:38Z
---

# Episode: Durable per-agent identity mechanism retired in docs, per-session keys confirmed as shipped model

## Prior State

Docs described a durable per-agent sovereign keypair ('you are your key' as a permanent self), identity/memory/reputation that persists across hosts/sessions/devices/vendors, 'vendor-independent agent identity' as a moat, and ordinal slots (haiku/haiku1). Meanwhile the shipped code already used per-session minted keys (src/runtime.rs:45, src/util.rs:128) — docs contradicted code.

## Trigger

Docs follow-up (#318) to reconcile philosophy corpus + arch/RPC docs with the shipped channel/identity redesign. The agent verified against actual code and found the durable-key mechanism was dead — code already mints per-session keys from the machine root.

## Decision

Retired the durable-key *mechanism* everywhere in docs: sovereign per-agent keypair, cross-vendor persistence, ordinal identity labels. Replaced with the shipped model: sessions are ephemeral, each mints its own key from the machine root, shows up as @codename@host; standing = current NIP-29 channel membership with 10-min prune; what persists is the fabric (channels, roles, awareness). Kept 'citizen' as a standing/participation metaphor (redefined in glossary), not a durable-key mechanism. Kept the self-organization/shared-awareness product frame (reinforced, not gutted).

## Consequences

- 15 docs edited across product-spec corpus and architecture/RPC docs
- Ordinal labels (@haiku1) replaced with codename handles + per-session minted key descriptions
- Known tension left: daemon-design.md §8a/§8b still claims 'sibling sessions of one agent share a pubkey' which per-session keys inverted — filed as #322 for code-verified rewrite (not done blind in docs pass)
- Wire RPC method names (chat_read, chat_write, project_add) kept as-is per instructions
- Reply-envelope rationale (messages.author_session needed because pubkey alone can't address a reply) is now factually backwards under per-session keys — tracked as dedicated follow-up

## Open Tail

- #322: daemon-design §8a/§8b reply-envelope rationale contradicts per-session key model — needs code-verified rewrite, not blind docs edit
- Whether messages.author_session column and reply-degradation logic are still needed when each session has a unique pubkey

## Evidence

- transcript lines 3983-4000
- transcript lines 4009-4036

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-b70718e17221-cb4e41ba-2-durable-per-agent-identity-mechanism-retired.json`](transcripts/2026-07-09-b70718e17221-cb4e41ba-2-durable-per-agent-identity-mechanism-retired.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-2-durable-per-agent-identity-mechanism-retired.json`](transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-2-durable-per-agent-identity-mechanism-retired.json)

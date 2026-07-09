---
type: episode-card
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: reversal
status: active
subjects:
  - durable-identity-retirement
  - per-session-keys
  - self-organization-frame
  - citizen-redefinition
  - docs-reconciliation
supersedes:
  - 2026-06-29-1-session-identity-model-from-patch-after
related_claims: []
source_lines:
  - 3976-4001
  - 4002-4006
  - 4087-4093
captured_at: 2026-07-09T16:06:52Z
---

# Episode: Durable identity mechanism retired in docs — self-organization frame kept

## Prior State

Documentation described a durable per-agent sovereign keypair ("you are your key"), ordinal identity slots (`haiku`/`haiku1`), and identity/memory/reputation/relationships that "persist across hosts/sessions/devices/vendors" as a moat. The "citizen" concept rested on this durable-key mechanism, framing agents as having a permanent self.

## Trigger

The channel/identity redesign shipped per-session ephemeral keys (each session mints its own key from the machine root, shows up as `@codename@host`), making the durable-key mechanism documentation factually backwards. Docs reconciliation (#318) was needed to align the philosophy corpus and architecture docs with shipped code.

## Decision

Retired the durable-key *mechanism* everywhere in docs: sovereign per-agent keypair, ordinals, cross-vendor persistence, "vendor-independent agent identity" as a moat. Kept the self-organization/shared-awareness *aspirational frame* (which the redesign reinforces rather than contradicts). "Citizen" deliberately retained as a standing/participation metaphor (redefined in glossary), not as a durable-key mechanism. Sovereignty survives as a user/machine-root property, not a per-agent one. Standing = current NIP-29 channel membership with 10-min prune.

## Consequences

- 15 docs edited across product-spec corpus and architecture/RPC docs
- Product-spec corpus updated: first-principles.md (principles #1, #2, #7), glossary.md (Agent/Citizen/Host-Body/Floor/Provenance), value-layers.md, vision.md, roadmap.md, prior-art.md, bets-and-open-questions.md (Q8 marked resolved → per-session), and others
- Architecture/RPC docs updated: CLI reader lists changed (chat read→channel read, channels list→channel list), ordinal labels replaced with codename handle + per-session minted key, invite→channel add, tui+mcp added to surface lists
- Deeper doc↔code contradiction surfaced but deliberately not fixed: daemon-design §8a/§8b still says sibling sessions share a pubkey (now inverted by per-session keys) — tracked as #322

## Open Tail

- #322 — doc↔code contradiction on shared-pubkey rationale needs code-verified rewrite (see separate finding)

## Evidence

- transcript lines 3976-4001
- transcript lines 4002-4006
- transcript lines 4087-4093

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-b70718e17221-cb4e41ba-2-durable-identity-mechanism-retired-in-docs.json`](transcripts/2026-07-09-b70718e17221-cb4e41ba-2-durable-identity-mechanism-retired-in-docs.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-2-durable-identity-mechanism-retired-in-docs.json`](transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-2-durable-identity-mechanism-retired-in-docs.json)

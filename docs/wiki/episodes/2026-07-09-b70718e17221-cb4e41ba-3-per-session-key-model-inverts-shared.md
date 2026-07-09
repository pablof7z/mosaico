---
type: episode-card
date: 2026-07-09
session: b70718e1-7221-496a-bc81-1ae5d1706296
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/b70718e1-7221-496a-bc81-1ae5d1706296.jsonl
salience: root-cause
status: active
subjects:
  - per-session-keys
  - shared-pubkey-inversion
  - reply-envelope
  - author-session-column
  - routing-architecture
supersedes: []
related_claims: []
source_lines:
  - 3998-4001
  - 4005-4006
  - 4016-4036
  - 4089-4093
  - 4152-4154
captured_at: 2026-07-09T16:06:52Z
---

# Episode: Per-session key model inverts shared-pubkey reply-envelope rationale

## Prior State

Architecture docs (daemon-design §8a/§8b, reply-envelope rationale in fabric-architecture.md and fabric-architecture-implementation.md) asserted that "identity is (agent, machine) → sibling sessions of one agent share a pubkey, so the pubkey alone can't address a reply." This justified `messages.author_session` as a canonical column (derived from kind:30315 status or local runtime state) and the reply-degradation logic — because the author pubkey was ambiguous across sibling sessions.

## Trigger

Docs reconciliation agent discovered (verified against `src/runtime.rs:45` "the session's OWN minted keypair") that the redesign gives each session its own minted key, inverting the shared-pubkey assumption that underpinned the reply-envelope architecture rationale.

## Decision

Tracked as #322 — deliberately NOT fixed blind in a docs pass. The finding: per-session keys mean the pubkey DOES uniquely address a session, which cascades into whether `messages.author_session` and the reply-degradation logic are still needed or should be simplified. Correct fix requires verifying current routing/reply code, not guessing in a docs-consistency pass.

## Consequences

- Known tension in docs: daemon-rpc-surface.md now says per-session key/codename while daemon-design.md §8a still says shared per-agent pubkey — explicitly inconsistent
- The reply-envelope passages in fabric-architecture.md:180, fabric-architecture-implementation.md:76, and fabric-architecture-overview.md:72 all assert "sibling sessions share a pubkey" — now factually backwards
- If per-session keys make author_session redundant, the messages table schema and reply routing may be simplifiable — but this is a code-informed design decision, not a doc edit

## Open Tail

- #322 requires code-verified investigation of how reply-addressing and messages.author_session actually work now with per-session keys

## Evidence

- transcript lines 3998-4001
- transcript lines 4005-4006
- transcript lines 4016-4036
- transcript lines 4089-4093
- transcript lines 4152-4154

## Conversation

- Cleaned transcript (verbatim user words, abbreviated agent replies): [`transcripts/2026-07-09-b70718e17221-cb4e41ba-3-per-session-key-model-inverts-shared.json`](transcripts/2026-07-09-b70718e17221-cb4e41ba-3-per-session-key-model-inverts-shared.json)
- Raw transcript (verbatim user words, full agent replies): [`transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-3-per-session-key-model-inverts-shared.json`](transcripts/raw/2026-07-09-b70718e17221-cb4e41ba-3-per-session-key-model-inverts-shared.json)

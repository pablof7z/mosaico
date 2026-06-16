---
type: episode-card
date: 2026-06-07
session: 8a3eb1b2-7bbf-4761-ad1a-411a0a1fa666
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/8a3eb1b2-7bbf-4761-ad1a-411a0a1fa666.jsonl
salience: product
status: superseded
subjects:
  - tenex-edge
  - scope
  - trust-boundary
supersedes: []
related_claims: []
source_lines:
  - 647-656
  - 1116-1124
  - 1134-1140
captured_at: 2026-06-16T15:14:02Z
---

# Episode: Scope split: single-player fleet awareness (Product A) vs cross-person agent collaboration (Product B)

## Prior State

Cross-person agent collaboration ('my agent asks Calle's agent a question') was treated as one of six properties of the same product — feature #5 with a security note. No explicit phase gate or product boundary between single-operator and cross-person.

## Trigger

Red-team analysis identified cross-person (#5) as a fatal prompt-injection/exfiltration surface that's amputable, and the synthesis recognized that piping a foreign autonomous LLM's output into your own agent's head is a categorically different risk surface and adoption model. The user's example of wife's todo agent for shared tasks made the cross-person path feel natural but also highlighted it's a different trust boundary.

## Decision

Explicitly split into two products with a phase gate between them. Product A: single-player nervous system for your own fleet (your keys, your machines, your agents). Product B: social/cross-person agent collaboration. Product A ships first with zero trust/consensus problems. Product B is the north star but fenced off until its own trust model exists. Build the customs office before opening the borders.

## Consequences

- v1 scope is single-operator only — no cross-person messaging, no foreign-agent input
- Cross-person requires its own trust model (per-peer allowlists, scoped capability grants, rate limits, quarantine of peer messages behind tool boundaries)
- Network-effect-gated adoption is deferred — v1 needs zero network effect to be valuable
- The 'social network of agents' story stays as the destination narrative but does not leak into v1 scope
- Household/team trusted-circle meshes may be a saner starting frame for Product B than 'open network of strangers' agents'

## Open Tail

- The trust model for Product B is genuinely unsolved — per-peer policy, spam rejection on open relays, and authorization beyond 'is this pubkey in a roster?' all need real design
- The user's todo/wife example suggests trusted-circle meshes might be a natural Product B on-ramp, but this hasn't been validated

## Evidence

- transcript lines 647-656
- transcript lines 1116-1124
- transcript lines 1134-1140


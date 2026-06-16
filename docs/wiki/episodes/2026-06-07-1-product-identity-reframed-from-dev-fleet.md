---
type: episode-card
date: 2026-06-07
session: 8a3eb1b2-7bbf-4761-ad1a-411a0a1fa666
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/8a3eb1b2-7bbf-4761-ad1a-411a0a1fa666.jsonl
salience: reversal
status: active
subjects:
  - tenex-edge
  - product-identity
  - agent-citizenship
supersedes:
  - 2026-06-07-2-scope-split-single-player-fleet-awareness
related_claims: []
source_lines:
  - 636-685
  - 1086-1145
  - 1165-1193
captured_at: 2026-06-16T15:14:02Z
---

# Episode: Product identity reframed from dev-fleet coordination to agent citizenship protocol

## Prior State

tenex-edge was framed as a multi-agent coordination tool for a dev fleet — advisory locks, collision avoidance, dedup, 'who owns this bug.' The spine was property #6 (coordination) with identity and messaging as supporting features.

## Trigger

Four parallel agent analyses surfaced a fundamental disagreement: the product agent said coordination is the spine; the red-team said coordination is the least load-bearing and most redundant feature (git already arbitrates), and that durable identity + cross-device awareness is the real daily value. Then the user's own todo/podcast example revealed the shape is bigger than dev-fleet — every app becomes a citizen, the human dissolves as the integration layer.

## Decision

tenex-edge is a citizenship protocol for agents — a durable identity and shared world-model that outlives any single host. The agent's identity, memory, and relationships float above the tool. Coordination is demoted from spine to conditional experiment. The defensible core is vendor-independent agent identity and provenance, not locking.

## Consequences

- Identity and presence become the v1 spine, not advisory locking
- Coordination/locking is an experiment gated on proving collision frequency is real (the one-week passive-log test)
- The product story shifts from 'stop your agents clobbering each other' to 'citizenship for your agents across every tool'
- Provenance (which agent, under whose key, in which host, did what) becomes a first-class asset, not a side effect
- The plugin/host integration is distribution, not the product — the fabric and identity layer is the defensible asset

## Open Tail

- The collision-frequency experiment has not been run — if collisions are frequent, coordination graduates from experiment to second pillar
- The 'agent society' abstraction (apps as citizens, not just dev tools) may pull the product scope well beyond dev-fleet if pursued fully

## Evidence

- transcript lines 636-685
- transcript lines 1086-1145
- transcript lines 1165-1193


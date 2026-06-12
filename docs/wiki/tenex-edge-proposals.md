---
title: Tenex-Edge Proposals
slug: tenex-edge-proposals
topic: tenex-edge
summary: "The proposal (kind:30023) becomes the only human-facing artifact"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-10
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:40a4d401-2520-4781-b747-b0ef19594bed
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
---

# Tenex-Edge Proposals

## Proposal-Centric Inbox

The proposal (kind:30023) is a tool agents choose to use, not a system-enforced gate; there is no centrally-planned state machine. The product direction is proposals-primary with an optional drill-in to see inside projects/conversations — no version-chain or section-approval ceremony. Notes and approvals are plain natural-language kind:1 content with no structured machine-tags like 'approve' or 'proposal'; capable agents read the content and act. The user's review of a proposal is conveyed by the arrival of their plain kind:1 note — there is no separate 'reviewed' event type. A note on a proposal is an owner-signed kind:1 routed to the exact session that generated the proposal — reusing the targeted-session delivery mechanism. Capable agents should not be over-prescribed with structured instruction tags or protocol ceremony; plain natural language in the content is sufficient for them to understand and act. (Previously: The proposal becomes the only human-facing artifact with a state machine inbox.)

<!-- citations: [^ab999-10] [^ab999-19] [^ab999-53] [^ab999-61] [^ab999-71] -->
## M1 Substrate

The tenex-edge M1 substrate consists of two additions: tenex-edge propose (agent publishes a kind:30023 + canonical record) and inbound admission of owner-signed notes (accepting a human-nsec-signed kind:1 targeting a session).

`tenex-edge propose` publishes a kind:30023 long-form event signed by the agent's identity with routing tags (`d`, `title`, `h`=project, `session-id`, `p`=owner) and a canonical thread record. The command accepts `--title`, `--message`, `--thread` (for root event-id), and `--d` (for stable-id/revisions). The `--thread` argument becomes an NIP-10 `e` tag with relay and marker `root` linking the proposal to the originating conversation. The `--d` flag allows revising an existing proposal at the same naddr; omitting it mints a new one. It automatically stamps tags `h` (project), `p` (owner), and `session-id` on the published event. The `propose` command must never include an `agent` tag. The `propose` CLI command (which publishes kind:30023 long-form events) was accidentally dropped in the `98582fa` refactor enforcing code file size limits and must be restored.

The propose command works without an active session, falling back to the cwd for project and `TENEX_EDGE_AGENT`/`--agent` for the slug. When a live session is available, the propose command uses it to determine project and slug and stamps a `session-id` tag on the event; when no session is available, the `session-id` tag is omitted.

The remote-first loop is: agent publishes a kind:30023 proposal (owner-tagged) → phone sees it → phone publishes a plain owner-signed kind:1 note to the relay → daemon admits it via the ownership gate and delivers it to the exact session that authored the proposal.

The `propose` tool publishes kind:30023 events to configurable relays. The architecture report describing the system is itself published as a kind:30023 event via the `propose` tool it describes, dogfooding the tool onto relay.primal.net at a persistent naddr.

A real claude-haiku agent created a project and published a real kind:30023 proposal via the `propose` tool on the production fabric, and real Claude↔Codex review chatter flowed via kind:1 messages routed by membership with slug from kind:0.

<!-- citations: [^ab999-20] [^ab999-36] [^ab999-37] [^ab999-38] [^ab999-54] [^ab999-72] [^40a4d-11] [^56f9f-6] [^56f9f-11] [^56f9f-13] [^56f9f-17] -->
## Product Direction Artifact

The product direction is synthesized into tenex-off/Plans/product-direction.md with locked decisions and an M1 spec. <!-- [^ab999-21] -->

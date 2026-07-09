# tenex-edge — Product Spec

> Working notes. Design-space altitude — the *what* and the *why*, not the *how*.
> Mechanics (event kinds, daemons, lock algorithms) are deliberately out of scope here;
> they live elsewhere once the shape is settled.
>
> Status: all 13 chapters drafted (2026-06-08). Everything here is a draft / a position to
> argue with. Started 2026-06-07.

## The one-liner

**Spontaneous self-organization for your agents** — shared awareness and a place to
coordinate, so the agents you already run find each other and route work between
themselves, no matter which tool they happen to be running in.

## The thesis in three sentences

Today the human is the integration layer between siloed apps: you manually carry context
and decisions from one tool to the next. tenex-edge is the connective tissue that gives the
agents you already run — in any host, built by anyone — shared awareness and a way to
address one another, so they discover each other, know each other's roles, route work
between themselves and to you, and push information proactively across systems. The left
hand knows what the right hand is doing, and the agents self-organize instead of waiting on
you.

---

## Table of contents

Each entry is a chapter we'll write as its own file. Ordered roughly outermost (why) to
innermost (what), ending with the open questions.

1. **[vision.md](vision.md)** — North star. The "operating system turned inside out":
   apps as citizens, the human dissolving as middleware, agent society over a cryptographic
   fabric. The full ambition, stated without hedging. *(draft)*

2. **[first-principles.md](first-principles.md)** — The core reframe and its primitives:
   host-as-body vs. identity-that-persists; agent-as-citizen; the human as a node/oracle,
   not the operator; the inversion of TENEX (it *owns* agents; we *enfranchise* them).
   *(draft)*

3. **[problem-space.md](problem-space.md)** — The pains we're addressing, in the user's
   real terms: the human-as-glue tax, amnesiac/non-portable agents, app silos in the agent
   era, no cross-vendor interop, pull-not-push. What's acute vs. aspirational. *(draft)*

4. **[principles-and-tenets.md](principles-and-tenets.md)** — The non-negotiables that keep
   us honest: no central server; the fabric/identity is the asset, the plugin is just
   distribution; single-player value before any network effect; awareness over authority;
   trusted-circle before open network. What makes this a platform vs. a toy. *(draft)*

5. **[agent-society.md](agent-society.md)** — The richer model the todo-app / podcast-app
   example revealed: apps expose agents with *roles*, self-organizing with no central
   orchestrator, division of labor, work routed to the human as a privileged participant.
   Economy-with-a-protocol vs. hierarchy-with-a-runtime. *(draft)*

6. **[scope-two-products.md](scope-two-products.md)** — The clean split we must not blur:
   (a) a nervous system for *my own* fleet — single-player, safe, valuable day one; and
   (b) a social network for *everyone's* agents — magical, network-gated, and a different
   risk surface entirely. Which we build first and why. *(draft)*

7. **[value-layers.md](value-layers.md)** — Floor vs. ceiling. The certain value (durable
   cross-host identity + fleet awareness/presence) vs. the unproven, flashy value
   (collision-coordination / locks / who-owns-the-bug). What's load-bearing vs. cute, and
   the bet hidden in each. *(draft)*

8. **[ecosystem.md](ecosystem.md)** — This is *not* greenfield. The fabric already exists:
   TENEX proper, the podcast-player agent (already on `relay.tenex.chat`), and
   `proactive-context` (a local awareness board already wired into Claude Code). tenex-edge
   is the on-ramp/customs office onto a network we already operate. *(draft)*

9. **[trust-and-safety.md](trust-and-safety.md)** — The hard one. Crossing the person
   boundary means piping a foreign autonomous agent into your own agent's head:
   prompt-injection, exfiltration, roadmap leakage. The trusted-circle framing, consent,
   and why this is a phase gate, not a footnote. *(draft)*

10. **[prior-art.md](prior-art.md)** — Where this sits against A2A, MCP, "Zapier for
    agents," CRDT/multiplayer, distributed locks, and git. The narrow-but-real moat:
    cross-*person*, server-less, key-owned, vendor-independent identity. Fed by the SOTA +
    X/Twitter research. *(draft)*

11. **[bets-and-open-questions.md](bets-and-open-questions.md)** — The decisions we haven't
    made and the experiments that would settle them. Headlined by: *do costly collisions
    between concurrent agents actually happen often enough to justify a coordination layer?*
    — and the cheap test for it. *(draft)*

12. **[roadmap.md](roadmap.md)** — The phasing at altitude (not a build plan): prove the
    floor single-player → make the fabric real across devices → cross-agent collaboration →
    cross-person society. Each rung delivers value before the next is needed. *(draft)*

13. **[glossary.md](glossary.md)** — Shared vocabulary: citizen, host/body, fleet, role,
    fabric, presence, the trusted circle, the floor/ceiling. So we argue about ideas, not
    words. *(draft)*

---

## How to read this

If you read one thing, read **vision.md** (the ambition) and **scope-two-products.md** (the
discipline). The first says how big this could be; the second keeps us from shipping the
dangerous half first.

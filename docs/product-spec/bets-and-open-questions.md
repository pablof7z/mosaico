# Bets & Open Questions

> Status: draft, 2026-06-08. The decisions we haven't made and the experiments that would
> settle them. This is the most *alive* chapter — it should change as we learn. Each item
> has the question, why it matters, and (where we have one) the cheapest way to get an answer.

## The headline question

### Q1 — Do costly collisions between concurrent agents actually happen often enough to justify a coordination layer?
**Why it matters:** this single unknown decides whether the flashy ceiling feature
(collision-coordination / locks) is a real pillar or a demo that rarely fires. If collisions
are frequent and expensive, coordination earns a place. If they're rare, the project
collapses cleanly to its (certain, valuable) floor and we've saved the hardest distributed-
systems work. (See [value-layers.md](value-layers.md).)

**The cheap test:** pure passive logging for ~a week across real concurrent agent sessions —
*no coordination logic at all.* Record `(agent, path, timestamp)`. Then count: how often did
two agents touch the same path within a plausible conflict window? Costs a day to build, a
week to run, zero distributed-systems work.

**Decision criteria:**
- Near-zero collisions → coordination is a demo, not a feature. Rescope to identity +
  awareness + messaging and re-evaluate.
- Frequent, costly collisions → there's a real problem; build *advisory* coordination on top
  (never authoritative — git stays the source of truth).

This is the first experiment to run, before writing any coordination machinery.

## Scope & sequencing

### Q2 — How far into the "trusted circle" do we go before calling it Product B?
The household case (spouse's todo agent) is conceptually part of A's umbrella but technically
crosses a key boundary. Where exactly is the line between "still single-player-ish, safe" and
"now it's cross-person, gate it"? (Relates to [scope-two-products.md](scope-two-products.md)
and [trust-and-safety.md](trust-and-safety.md).)

### Q3 — Is the floor valuable enough *alone* to be the whole v1, or does it need one
"magic" cross-agent moment to land emotionally?
Identity + awareness is certain but possibly *undramatic*. Does v1 need the todo-agent-raises-
a-hand-to-the-human flow (a within-fleet routing moment) to feel alive, even before any
cross-person or coordination feature? Leaning yes — the routing/role moment may be the
cheapest "wow" that's also load-bearing.

## Trust & authorization

### Q4 — What's the minimum viable authorization model for the first trusted-circle step?
Authentication is free; authorization is the unsolved work (see
[trust-and-safety.md](trust-and-safety.md)). What's the *smallest* consent model that's
genuinely safe for the household/team case — and can it be scoped, revocable, and default-deny
without becoming unusable?

### Q5 — Can "peer input is data, not instructions" actually be enforced across hosts we
don't control?
The cardinal safety rule is easy to state and hard to guarantee when the agent runs inside
someone else's host whose context-injection we only partly control. Is the boundary
enforceable, or only encourageable? This may bound how far cross-person can safely go.

## Substrate & architecture

### Q6 — Is Nostr the right substrate, or a constraint we're rationalizing?
The sovereignty/no-central-anything property is the product, and Nostr gives it for free. But
its eventual-consistency, no-ordering, best-effort nature is a real tax (it's why we're
awareness-over-authority). Keep asking: if the sovereignty property stopped being central,
would we still choose this? (See [prior-art.md](prior-art.md).)

### Q7 — How brittle is living as a guest in hosts we don't control?
Each host's extension surface is a moving target. Claude Code is genuinely hookable (and the
best case); others are thinner and more volatile; some hosts aren't deeply hookable at all.
What's the real maintenance tax, and which hosts are worth supporting at which depth? (Tenet
#7: advertise the tier honestly; never bet the product on one host.)

### Q8 — Identity granularity: one identity per person? per agent? per role? per session? — *resolved*
Settled toward **per session**: each session mints its own key from the machine's root secret.
Its npub is permanent and its friendly dashed handle, such as `@quill-codex`, is a reclaimable lease; there is no durable per-agent keypair
and no ordinal slots. Standing isn't carried by a key at all — it's *current channel
membership*, pruned when a session goes quiet. What stays open is reputation: with identity
deliberately ephemeral, any durable reputation must attach to the person or the channel, not
the agent-session — and how that accrues is unsettled.

## Strategy

### Q9 — Plugin-as-distribution vs. plugin-as-trap.
We live inside tools we don't control. The defense is "own the fabric, not the feature"
(tenet #2). But is the plugin posture a durable wedge or a structural weakness — and is there
a point where we'd want a first-class surface of our own (a CLI, a thin native host for the
phone case) without becoming TENEX?

### Q10 — What's the actual beachhead user, and is it the dev power-user or the
multi-app individual?
The brainstorm leaned "solo dev running Claude Code + Codex." But the todo/podcast example
suggests the richer beachhead might be "the individual who already has fabric-capable agents
across several apps." These imply different first features. Unresolved.

---

## How to use this chapter

When we make one of these calls, move it out of "open" and record the decision *and the
reasoning* — ideally promoted into the relevant chapter. The goal is that this file shrinks
over time as questions become positions, leaving only the genuinely live unknowns.

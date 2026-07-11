# Value Layers — Floor vs. Ceiling

> Status: draft, 2026-06-08. This chapter is principle #6 ("awareness over authority") and
> tenet #8 ("coordination is an experiment") applied to the feature set. It exists to settle
> a real disagreement that surfaced in the brainstorm: *is the core value coordination, or
> is it identity + awareness?*

## The disagreement, stated fairly

When we put four independent perspectives on this idea, they split — and the split is the
most useful output we got:

- **The product instinct:** the spine is *coordination* — two agents stop clobbering each
  other and start covering for each other. Locks, dedup, who-owns-the-bug. It's the demo
  that makes people gasp.
- **The skeptic's rebuttal:** that's the *least* load-bearing and most over-claimed part.
  Git already arbitrates collisions. Real collisions between your concurrent agents are
  probably rare. The "boring" stuff — identity, presence, cross-host messaging — is
  where the daily value actually lives, at a fraction of the risk and complexity.

The resolution: **they're both right, about different layers.** One is the floor; the other
is the ceiling. The mistake would be to let the ceiling masquerade as the foundation.

## The floor — certain value

These are valuable on day one, single-player, with no consensus problem and no trust
problem. They're the things a *society* needs to exist at all (see
[agent-society.md](agent-society.md)), which is why they're foundational rather than optional.

- **Per-session identity + channel membership.** Each session has a permanent npub plus a short
  leased handle and joins the shared fabric the moment it starts — instantly addressable and trusted, with
  no setup. The root of everything; nothing else works without it.
- **Presence & awareness of your own fleet.** Knowing what your agents are doing, across
  devices — without going and asking each one.
- **Cross-host / cross-device messaging between your own agents.** Work and context that
  follow *you*, not the machine. The end of re-explaining yourself to every new session.
- **Roles & routing within your fleet.** Agents knowing each other's roles so a coding agent
  can hand a decision to the todo agent and the todo agent can escalate to you.

This is the part that is **certainly real**. If we built only this, we'd have a valuable
product and the disagreement above would be moot.

## The ceiling — high-wow, unproven

- **Collision detection / advisory coordination.** Two agents notice they're on the same
  file/bug and coordinate (wait, hand off, co-fix, decide ownership).

This is the headline demo and the seductive feature. It's also resting on an **untested
premise**: that costly collisions between concurrent agents happen *often enough* to justify
a coordination layer — and that pre-collision awareness beats the at-merge truth git already
provides. We do not know this is true. It might be a genuine daily pain or it might be a
demo that rarely fires in real life.

So we treat it as a **hypothesis to test cheaply**, not a pillar to assume. (The test and
the decision criteria live in [bets-and-open-questions.md](bets-and-open-questions.md). The
short version: passively log what your real agents touch for a week and *count the
collisions* before building any coordination machinery.)

And even if it proves real, it stays **advisory** — it informs; it never claims authority git
doesn't grant it (principle #4).

## The two defensible bets

Beyond floor and ceiling, two things rise above as the *long-term moat* — they survive even
if a host vendor ships native coordination tomorrow and eats the locking feature:

- **A vendor-independent coordination fabric.** Shared awareness and a place to coordinate
  that the user owns outright, spanning hosts nobody controls and surviving any single session,
  host, or vendor. Nobody is building this for *heterogeneous* hosts. This is the milkshake;
  the plugin is the straw (tenet #2).
- **Provenance.** A signed record of which session, under whose machine root, in which host,
  produced a given piece of work. A shrug today; a necessity once agents write most of
  everything.

Both flow from **the user-owned fabric**, not from coordination. That's the tell for where the
center of gravity belongs.

## The trap to avoid

Letting the *exciting* feature (collision-coordination) become the *identity* of the project
when the *certain* feature (shared awareness + fleet messaging) is the one guaranteed to matter.
Demo with the ceiling if it tests out; *build on* the floor regardless.

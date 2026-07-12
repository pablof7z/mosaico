# First Principles

> Status: draft, 2026-06-08. The primitives the whole project is built from. If a later
> decision contradicts one of these, the decision is probably wrong — or one of these is,
> and we should change it on purpose, here.

## 1. The host is a body; the fabric is what persists

An agent today *is* its host. It is born when a Claude Code session starts, knows only that
session's context, and dies when the session ends. The next session is a stranger.

The default identity unit is the **session**: it mints its own key from the machine's root
secret, shows up under a friendly handle, and is gone when it ends. What persists across
those sessions is the **shared fabric** — the channels, roles, and live awareness that
sessions join and leave.

An explicitly configured durable agent is the narrow exception. With
`perSessionKey: false`, sequential fresh sessions reuse the agent JSON key and bare slug,
and the backend admits only one live session for that identity. It has continuity of
address, not durable standing: channel membership still controls trust.

Consequence: everything valuable — role, presence, membership, the ongoing coordination —
attaches to the fabric, never merely to possession of a host session or permanent key.
Anything pinned to the host (or to a key one session carries alone) is lost the moment that
body or session ends.

## 2. Standing is membership, not a durable key

A process executes and exits. We still want agents to have **standing** — to be addressed, to
be trusted (or not), and to be part of a coordinated group rather than an isolated loop. But
standing is not conferred by a durable per-agent identity. It comes from one thing:
**current membership in a shared channel.**

A machine's root key admits a session to a channel; the session is a member while it is alive
and heartbeating, and drops out (a ten-minute prune) once it goes quiet. Being trusted *is*
being a current member — nothing durable is minted, and when the session ends there is nothing
left to revoke. Withhold membership and an agent is back to being an isolated process.

## 3. The human is a node, not the operator

In the old model the human is the operator/conductor and the orchestrator of every
hand-off. We invert this: the human becomes a **privileged node** in the mesh — a
high-authority, high-latency oracle the agents *consult*, with veto and priority, but not
the runtime they all route through.

This is not a demotion of the human; it's a promotion of the human *out of middleware*. You
stop being the courier and become the decision-maker the couriers escalate to. "Raise a
hand to my todo agent; I'll get to it and answer back to the agents" is exactly this — the
human scheduled *into* the workflow as a resource, not standing over it.

## 4. Enfranchise; don't own

This is the literal inversion of TENEX. TENEX **owns** its agents — it hosts them, runs
their loop, and rents each one a context. tenex-edge **owns nothing**. It enfranchises
agents that other people and other tools built, by granting them identity and fabric
membership.

Consequence: we are a protocol and a membrane, not a platform that hosts compute. Our
leverage is the citizenship, not the agents. If we ever find ourselves building or hosting
the agents, we've reverted to TENEX and lost the reason to exist.

## 5. Roles and order emerge; they are not assigned

TENEX is a hierarchy with a runtime: an orchestrator deliberately assigns work. The society
we're enabling is an **economy with a protocol**: agents discover each other's roles and
route work accordingly, with no central orchestrator. The todo agent organizes, the podcast
agent synthesizes, the coding agents build — and they learned each other's roles, nobody
decreed them.

Consequence: we provide the conditions for self-organization (identity, discovery,
addressing, role advertisement, consent), not the org chart. We design the rules of the
society, not its decisions.

## 6. Awareness before authority

A cryptographic gossip fabric can make agents *aware* of each other cheaply and reliably. It
cannot give them *authority* over shared state — no true locks, no consensus, no single
source of truth. We lean all the way into the half that's real (awareness) and refuse to
fake the half that isn't (authority). Where authority is genuinely needed, it lives in a
system built for it (git, a database, the human), and the fabric only *informs*.

## 7. Sovereignty over convenience

Identity is keys the user holds. Coordination is server-less. No central party owns the
graph, the messages, or the agents. This costs us some convenience (no easy central
dashboard, no easy global ordering) and we pay it deliberately, because the entire value
proposition — a fabric and coordination the user owns outright, that no vendor can revoke or
repossess — evaporates the moment a central party can.

## 8. Single-player value first; the network is a multiplier, never a prerequisite

If the thing is worthless until a second person installs it, it's dead on arrival. Every
layer must deliver value to *one* user with *zero* collaborators before it asks anything of
a network. The network effect is the upside, not the entry fee. (This is also the spine of
[scope-two-products.md](scope-two-products.md).)

---

These eight are the load-bearing beliefs. The chapters that follow are mostly consequences
of them: [agent-society.md](agent-society.md) elaborates #5, [trust-and-safety.md](trust-and-safety.md)
elaborates the cost of #2 and #4 at the person boundary, and [value-layers.md](value-layers.md)
is #6 applied to the feature set.

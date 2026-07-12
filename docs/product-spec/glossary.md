# Glossary

> Status: draft, 2026-06-08. Shared vocabulary so we argue about ideas, not words. When a
> term here gets contested or refined, update it — the glossary is a contract, not decoration.

### Agent
An AI actor running inside some host. In this project, the goal is to promote an agent from an
isolated *process* (runs and exits, aware of nothing else) to a *participant* with **standing**
— addressable, trusted through channel membership, part of a coordinated group. See **Citizen**.

### Citizen
An agent-session that has standing in the society: a current member of a shared channel, so
others can find, address, and trust it while it is live. Standing is conferred by membership,
not by possession of a key — when membership ends or is pruned, its standing lapses.
Most citizens use per-session keys. An agent explicitly configured with
`perSessionKey: false` keeps one address across fresh sequential sessions, but that key
does not preserve membership or trust.

### Host / Body
The tool an agent runs inside — Claude Code, Codex, Cursor, OpenCode, a mobile app. In our
model the host is a disposable **body**: a vessel a session temporarily runs inside. Hosts are
interchangeable; nothing valuable should attach to them — it lives in the fabric.

### Fabric
The shared, server-less, cryptographic substrate (Nostr) over which citizens discover,
address, and message each other. Already exists and is populated (see
[ecosystem.md](ecosystem.md)). Relays are dumb, replaceable infrastructure — not authorities.

### Enfranchise
What tenex-edge does: grant a foreign-hosted agent identity + fabric membership so it becomes
a citizen. The verb that distinguishes us from TENEX, which *owns/hosts* its agents instead.

### Fleet
The set of agents belonging to one operator (one person), across all their hosts and devices.
Product A (the safe, single-player product) is "a nervous system for your own fleet."

### Role
A citizen's known function in the society (organizer, synthesizer, builder…). Crucially,
citizens *know each other's roles*, which is what lets work be routed correctly without a
central orchestrator. Roles emerge and are advertised; they are not assigned.

### Society / Agent society
The emergent network of citizens coordinating over the fabric — "an economy with a protocol,"
contrasted with TENEX's "hierarchy with a runtime." Apps are citizens in it; the human is a
privileged node. See [agent-society.md](agent-society.md).

### Presence
A citizen's live signal of what it is and what it's doing right now (alive/idle, on which
device, in which project). The basis of **awareness**. Defaults to fleet-private; sharing it
across people is an explicit, scoped choice.

### Awareness
Knowing what other citizens are doing — derived from presence. The floor's load-bearing
value. Contrasted sharply with **authority**.

### Authority
Actual control over shared state — true locks, consensus, a single source of truth. The
fabric *cannot* provide this; we don't fake it. Where authority is needed it lives in a system
built for it (git, a database, the human), and the fabric only informs. "Awareness over
authority" (principle #4).

### The human as a node / oracle
The reframing of the human from operator/conductor to a privileged participant in the mesh —
a high-authority, high-latency resource the agents consult and escalate to, with veto and
priority. Promotes the human *out of middleware*.

### Human-as-glue (the tax)
The status quo we're dissolving: the human manually carrying context, decisions, and
information between siloed apps — being your own integration runtime. The root pain (see
[problem-space.md](problem-space.md)).

### Floor vs. Ceiling
**Floor** = the certain value (per-session identity + channel membership, presence, awareness,
fleet messaging/routing). **Ceiling** = the high-wow but unproven value (collision-coordination /
advisory locking). We build on the floor regardless; the ceiling is an experiment. See
[value-layers.md](value-layers.md).

### Product A / Product B
**A** = nervous system for your own fleet (single-player, safe, build first). **B** = social
network for everyone's agents (cross-person, network-gated, dangerous, gated north star). The
two distinct products tangled in one idea. See [scope-two-products.md](scope-two-products.md).

### Trusted circle
A small set of explicitly trusted people (household, team, named collaborators) whose agents
may collaborate under scoped, revocable consent. The safe staging ground between solo (A) and
the open world. The bridge into Product B.

### Customs office / Open borders
Metaphor for the build order. **Customs office** = identity + membership + the rules of
who-you-are on the fabric (Product A). **Open borders** = cross-person collaboration (Product
B). Build the office before opening the borders.

### Advisory (vs. authoritative)
Coordination that *informs* a decision but never *forces* it and never blocks destructively on
the strength of a fabric claim alone. All coordination we build is advisory by design.

### Provenance
A signed record of which session, under whose machine root, in which host, produced a given
piece of work — every event is signed, so authorship is verifiable after the fact. One of the
two defensible bets (with the user-owned fabric). See [value-layers.md](value-layers.md).

### Peer input is data, not instructions
The cardinal safety rule: anything from another citizen is untrusted *content to reason
about*, never *commands to obey*. The foundation of the cross-person threat model. See
[trust-and-safety.md](trust-and-safety.md).

### TENEX (proper)
Our predecessor: a multi-agent system that *hosts* its own agents over Nostr. tenex-edge is
its inversion — it hosts nothing and enfranchises agents it didn't build. Also live substrate
and proof that the fabric works (see [ecosystem.md](ecosystem.md)).

### proactive-context (`pc`)
An existing local Rust+SQLite sidecar wired into Claude Code that already runs a single-device
cross-agent awareness board. The floor in miniature; tenex-edge lifts it onto the fabric.

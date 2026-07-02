# tenex-edge — Fabric Architecture (proposal)

> High-level architecture for the swap-seam. The load-bearing idea: **all data is
> read from one unified local store; *how* it was hydrated is irrelevant to its
> use.** A **Fabric Provider** (legacy-tag / nip29 / mls / a2a / …) is a write-side
> materializer that owns all of how-and-who — wire shape, membership/admission, lifecycle
> side-effects — and projects everything into canonical store rows. Readers query
> the store; nothing in a read path ever names a kind, a tag, a group, or a relay.

---

## 1. The core problem

The current `Codec` seam swaps *NIP layouts*, not *fabrics*. It traffics in
`nostr_sdk` types and fuses three unrelated concerns into one trait:

- **wire mapping** (domain event ↔ envelope),
- **subscription model** (`filters → Vec<Filter>`, relay-REQ-shaped),
- **admission control** (NIP-29 group create / lock / put-user, bolted into `legacy-tag`).

That fusion is why "a new codec" can only ever be another nostr codec, and why
NIP-29 — an *admission strategy* — leaks into an *event codec*. The fix is to cut the
seam along **concerns**, not along **kinds**.

Two observations drive the whole design:

1. **Membership is the hinge.** Whether to show a peer's presence or deliver a
   mention to an agent is one decision — *"is this pubkey a member?"* — but its
   **source** differs per fabric:

   | Fabric | "member" means | hydrated from |
   |--------|----------------|---------------|
   | nip29  | in the NIP-29 group | live `39002` members list (kept subscribed) |
   | mls    | in the MLS group | MLS group roster after invite/accept |
   | legacy-tag  | locally accepted | a future legacy-tag-owned local trust file |

   The **shape** is uniform (`is_member(project, pubkey)` + a change stream); the
   **source** is the provider's secret. Add a member from another machine → the
   nip29 provider's live subscription reflects it; nothing above notices *how*.

   The **enforcement locus** also differs — and this is what forces admission to be
   a domain-side gate rather than something we delegate to the fabric:

   | Fabric | membership enforced | by whom |
   |--------|---------------------|---------|
   | nip29  | server-side — relay rejects non-member writes (closed group) | the relay |
   | mls    | cryptographically — non-members cannot decrypt | the crypto |
   | legacy-tag  | client-side — a future legacy-tag provider filters inbound locally | us |

   **Principle:** the domain `is_member` gate is *always* consulted client-side;
   server/crypto enforcement is defense-in-depth, never a replacement. legacy-tag has
   no server enforcement at all, and even nip29 has un-scoped inbound paths (a
   direct p-tagged note reaches us via the `mentions_to` filter without the relay
   ever checking group membership). So the gate can never be skipped — which is
   exactly why it lives in the domain, above the provider seam.

2. **Lifecycle events have provider-specific side-effects.** "I run claude-code
   in a never-seen directory" is one domain event — `ProjectOpened` — that each
   provider *reacts* to differently:

   | Fabric | reaction to `ProjectOpened` |
   |--------|-----------------------------|
   | nip29  | create group `9007` → lock closed `9002` → put agent member `9000` |
   | mls    | create MLS group → invite agent key → await accept |
   | legacy-tag  | **no-op** — a "group" is just a `t`/`h` tag on each event |

---

## 2. Layer cake

```mermaid
flowchart TD
    subgraph HOST["Host adapters"]
        H1["Claude Code hooks / CLI"]
        H2["Codex"]
        H3["opencode"]
    end

    subgraph DOMAIN["Domain — abstract verbs & nouns (no kinds, no tags)"]
        direction LR
        PS["ProjectState plane<br/>roster · presence · status · project-meta"]
        CM["Communications plane<br/>send · inbox"]
        ADMIT["Admission / routing policy<br/>is_member? deliver? show?"]
    end

    SEAM{{"Fabric Provider trait — THE SWAP SEAM<br/>speaks DomainEvent + Scope only"}}

    subgraph PROV["Concrete providers (each owns its own wire types)"]
        direction LR
        P1["Nip29Provider"]
        P2["MlsProvider"]
        P3["LegacyTagProvider"]
    end

    subgraph WIRE["Wire / transport substrate"]
        R1["Nostr relays"]
        R2["MLS delivery service"]
    end

    HOST --> DOMAIN
    PS --> SEAM
    CM --> SEAM
    ADMIT --> SEAM
    SEAM --> P1 & P2 & P3
    P1 --> R1
    P3 --> R1
    P2 --> R2
```

**Rule of the seam:** everything *above* `SEAM` is written once and never edited
to add a fabric. Everything *below* is a self-contained provider. The domain's
vocabulary is `DomainEvent` + `Scope` (today's `SubScope`) — the two types that
are already genuinely transport-agnostic.

---

## 2a. The read model is the contract (the load-bearing principle)

**All consumption reads from one unified local store; *how* the data got there is
invisible to the reader.** A provider is a **write-side materializer** — it
subscribes to its fabric, decodes, admits, and **upserts canonical rows**.
Every consumer (CLI `who`/`inbox`/`list`, the channel adapter, hooks, context
injection) reads only the store. No reader ever holds a `Provider`, names a kind,
or touches the wire. This is CQRS, and it is exactly why the daemon can solely own
`state.db`: providers write, IPC clients read.

This store already exists — `~/.tenex/edge/state.db`, with `profiles`,
`peer_sessions`, `inbox`, `agent_status`, `project_meta`, … The architecture
**extends** it; it does not define a new one. And the
**single-writer materializer is the direct fix for the multi-writer `state.db`
corruption** already hit when ~16 per-session processes wrote concurrently: one
daemon owns the writer, every session/CLI is a read-only IPC client.

```mermaid
flowchart LR
    subgraph FABRICS["Fabrics — write-side, adapter-facing"]
        F1["legacy-tag"]
        F2["nip29"]
        F3["mls"]
        F4["a2a / invented / future"]
    end
    MAT["Provider = materializer<br/>decode · admit · derive · upsert"]
    STORE[("Unified read model — SQLite / state.db<br/>projects · agents+membership")]
    subgraph READERS["Readers — never touch the wire"]
        R1["CLI: who / inbox / list"]
        R2["channel adapter"]
        R3["hooks / context injection"]
    end
    F1 --> MAT
    F2 --> MAT
    F3 --> MAT
    F4 --> MAT
    MAT -- write --> STORE
    STORE -- query --> R1
    STORE -- query --> R2
    STORE -- query --> R3
```

**The canonical entities** (provider-agnostic — no kind, tag, or group-id in any
column a reader sees; a hidden `origin`/`wire_id` column may exist for the
*writer's* reconciliation only), mapped onto the real schema:

| Entity | Today's table(s) | Holds | Within |
|--------|------------------|-------|--------|
| project + metadata | `project_meta` (`project`, `about`, `updated_at`) | slug, `about` text | — |
| agents + identity | `profiles` (`pubkey`,`slug`,`host`) | identity card (slug, host, owners) | — |
| membership | `group_members`, canonical `membership` | which agents are online/routable in a channel | a project/channel |
| status | `peer_sessions`, `agent_status` | per-session activity, title, and history | a project/channel |

Most rows already exist; the materializer reframing keeps membership explicit in
the NIP-29 group/member tables.
entity.

**The message row must carry its own return envelope.** A reader that surfaces an
inbound message has to know *who to reply to* — and that means the exact sender
*session*, not just the author pubkey: sibling sessions of one agent share a
pubkey, so the author key alone can't address a reply. So `inbox.from_session`
is a canonical column derived from session pubkeys or local runtime state, never
from a session-specific wire tag. The **reply handle** is
then a *read-side derivation* over store rows — the session id when it resolves to
a known session, else `slug@project` — exactly the same shape as the `is_member`
read-query: a pure `SELECT`-time computation, never a trip to the wire. When a
fabric can't supply a sender session the handle degrades honestly to agent-level,
the same `Option`/derived concession as everywhere else.

**Three consequences that make "how we hydrate is irrelevant" true:**

1. **Multiple providers populate one store.** Project A on nip29 and project B on
   legacy-tag land in the *same* tables; a reader querying `list_projects()` cannot
   tell which fabric backed which row, and doesn't care.
2. **Every per-fabric difference lives behind the materialization seam.** The
   provenance axis, the enforcement-locus, the derived-vs-enumerated distinction
   (§3a) all describe *how the materializer fills a cell* — a reader sees a row or
   a `NULL`, never *why*. `Option`/divergence is the store's way of being honest
   when a fabric has no shared truth.
3. **Threads are a read-model entity even though no fabric has native threads.**
   *Deriving* thread structure (from reply-edges, `e`-tags, MLS message order) is
   a write-side materializer job; readers just `SELECT * FROM messages WHERE
   thread = ?`. This resolves the old "is Thread a wire noun?" question: no — it's
   a store noun the provider populates by whatever means its fabric allows.

So the swap-seam has two faces, and only one of them is ever in a reader's call
path:

- **Read face — the store schema.** Stable, provider-agnostic, the real contract.
- **Write face — the `Provider`.** Materializes inbound, publishes intents. Swap
  the fabric → swap the materializer; the schema and every reader are untouched.

---

## 3. The verbs — reads query the store, intents route to a provider

Verbs come in **two kinds**, and the distinction is *who is in the call path*:
**reads** are pure queries against the unified store (no provider, identical for
every fabric); **intents** are the only verbs that touch a provider (they publish
to the wire and reflect back into the store).

```mermaid
flowchart LR
    subgraph R["READS — query the store (provider-agnostic)"]
        r0["list_projects()"]
        r1["project_meta(project)"]
        r2["list_agents(project) + agent_meta"]
        r3["roster / is_member(project, pk)"]
        r4["presence / status(project)"]
    end
    subgraph I["INTENTS — route to the active provider"]
        i0["open_project(project)"]
        i1["send(to, project, body)"]
        i2["set_status / heartbeat"]
    end
    STORE[("unified read model")]
    PROV["active Provider"]
    R --> STORE
    I --> PROV
    PROV -- materialize --> STORE
```

- **Reads** are exactly the user-facing list — *which projects exist, who's in
  them, who's online, and what they're doing.
  recipient of each.* All are `SELECT`s. None know the fabric.
- **Intents** are writes: open a project, send a message, beat a heartbeat. The
  provider encodes the intent to its wire shape and (optimistically, or on echo)
  the materializer reflects it back as a row.
- **The admission gate lives on the write face, then becomes a read.** `is_member` is
  consulted *twice*: once at materialization time as an **admission predicate**
  (decode an inbound event → is the sender authorized → upsert or drop), and again
  at read time as a **query** over the membership rows (who may I show / route
  to). Both consult the same rows; neither touches the wire. One policy, one
  place — the store.

### 3a. Behind the materialization seam — the provenance axis

Everything in this subsection happens **on the write face, invisible to readers**.
It explains *how the materializer fills `project_meta` and `membership`* — the
reader just sees the resulting row (or a `NULL`). Just as membership had an
*enforcement-locus* axis, project metadata has a **provenance / authority** axis —
*where the description comes from, and whether it is shared truth* — which differs
per fabric:

| Fabric | project *list* source | *description* source | authority / consistency |
|--------|----------------------|----------------------|-------------------------|
| nip29  | groups the agent belongs to (reverse of `39002`) / relay group enumeration | relay-authored `kind:39000` group metadata | **canonical & shared** — one source, every machine agrees |
| mls    | MLS groups in local state | group-context extension / metadata message | **member-authored**, cryptographically scoped to the group |
| legacy-tag  | *derived* — observed `h`/`t` tags + local list of dirs run in | **none native** → local descriptor file or a self-published note | **client-local** — two machines may disagree; eventually-divergent |

The sharp edge is **legacy-tag**: a "group" is just a tag, so there is no native
carrier for a description and no authoritative project registry. Two consequences
the domain must absorb:

- **The list is *derived*, not *enumerated*.** For legacy-tag, `list_projects()` is
  reconstructed from observed events (which `h`/`t` tags have we seen?) plus a
  local record of directories opened — never a server-side directory listing.
- **Description is `Option`, and may be local-only.** The domain types
  `description: Option<String>` and tolerate per-machine divergence. This is the
  exact analogue of legacy-tag's client-side membership enforcement: not a flaw in the
  abstraction, but the abstraction faithfully surfacing that the fabric has no
  shared truth here.

**Hydration mode is the materializer's business too** — *pull vs. live*. nip29/mls
can one-shot **fetch** the `39000` metadata *or* **subscribe** to it (it's
replaceable) and re-upsert on every change, so a description edited on another
machine propagates by simply updating the store row — and the reader's next
`SELECT` reflects it with zero changes anywhere above the seam. legacy-tag uses whatever
local mechanism applies (file watch, or re-derive from the event stream). Either
way the reader sees only the current row; "a new project appeared on the fabric"
is just an `INSERT` it will observe on its next query (or via a store-level
change-notify, never a fabric subscription).

---

## 4. The Fabric Provider seam (SRP decomposition)

A `Provider` is **one cohesive object per fabric** that bundles four
single-responsibility capabilities. Splitting them keeps each concern testable
and prevents the current "codec also does admission" fusion.

```mermaid
flowchart TD
    PROVIDER["FabricProvider<br/>(Nip29 · Mls)"]
    PROVIDER --> L["① Lifecycle reactor<br/>react(ProjectOpened, AgentJoined…)<br/>→ native side-effects"]
    PROVIDER --> M["② Materializer<br/>composes ③+④ → admit · derive<br/>· upsert canonical rows into the store"]
    PROVIDER --> W["③ Wire codec<br/>DomainEvent ⇄ raw envelope<br/>(bytes / event / MLS app-msg)"]
    PROVIDER --> D["④ Delivery<br/>publish(raw envelope) · subscribe(scope)→raw stream<br/>owns REQ-filters / gossip / MLS-stream"]
```

| # | Capability | Responsibility | Must **not** |
|---|------------|----------------|--------------|
| ① | **Lifecycle** | Turn a domain lifecycle event into provider-native setup (create group, invite, or no-op). | Decide *when* a project opens (that's the host/daemon). |
| ② | **Materializer** | **Composes ③ and ④:** consume ④'s inbound stream, decode via ③, then own *only* admission, and upsert of canonical rows — membership, project list & metadata, agents. The store is the read contract; this fills it. | Subscribe or decode *itself* (that's ④ and ③), or answer reads (readers query the store directly; the materializer never sits in a read path). |
| ③ | **Wire codec** | Pure, symmetric ser/de of the five+ `DomainEvent` nouns to its envelope. | Open subscriptions or manage groups. |
| ④ | **Delivery** | Connect/auth, publish raw envelopes, and stream raw inbound envelopes for a `Scope`. Owns whatever fetch model the fabric uses. | Decode, derive, apply admission, or know domain meaning. |

The runtime only ever talks to one active provider interface. Swapping fabric =
swap the provider constructor (or a small enum of providers until a truly
object-safe async trait is needed). The `filters`-shaped subscription model
disappears from the public seam — it becomes a private detail of the nostr
delivery impl, so a push/gossip/MLS delivery model is now expressible.

---

## 5. Walkthrough — "a brand-new project spins up"

Same domain trigger, three provider reactions. The host adapter emits
`ProjectOpened(dir)`; everything downstream is provider-private.

```mermaid
sequenceDiagram
    participant CC as Claude Code (host)
    participant DOM as Domain / daemon
    participant P as Active Provider
    participant FAB as Fabric
    participant STORE as Unified read model

    CC->>DOM: ProjectOpened(new dir)
    DOM->>P: lifecycle.react(ProjectOpened)

    alt nip29 provider
        P->>FAB: create group 9007 (h = dir slug)
        P->>FAB: edit-metadata 9002 (closed + public)
        P->>FAB: put-user 9000 (agent = member)
        %% subscribe 39002 members keeps admission live
        P->>FAB: subscribe 39002 members
    else mls provider
        P->>FAB: create MLS group
        P->>FAB: invite agent key
        FAB-->>P: agent accepts → roster updated
    else legacy-tag provider
        P-->>P: no-op (group == t/h tag; nothing to create)
    end

    Note over P,FAB: thereafter the materializer just keeps the store current
    P->>FAB: subscribe (membership, metadata, …)
    FAB-->>P: events
    P->>STORE: upsert rows (members, project_meta)
```
*(`STORE` = the unified read model; the host/CLI reads it directly, never `P`.)*

Then a human messages the agent — note the **send path** and the **inbound path**
both terminate at the store, and the reader is never in the loop:

```mermaid
sequenceDiagram
    participant ME as Operator
    participant P as Active Provider
    participant FAB as Fabric
    participant STORE as Unified read model
    participant RD as Reader (CLI / hook)

    ME->>P: send(to = claude, project, body)
    P->>FAB: publish (provider's wire shape)
    P->>STORE: upsert message row (optimistic)

    Note over FAB,P: inbound side
    FAB-->>P: inbound event
    P->>P: is_member(sender)?  (admission)
    P->>STORE: upsert message + recipient (if admitted)

    STORE-->>RD: rows
```

The admission check (`is_member?`) is identical logic for all three fabrics; only
the **source rows** it reads were filled differently. Add a pubkey as a member
from another computer → the nip29 materializer's live `39002` subscription upserts
the `membership` table → the next admission check and the next reader `SELECT`
both reflect it, with zero changes above the store.

---

## 6. Implementation ladder

The behavior-preserving phase ladder and validation commands live in [fabric-architecture-implementation.md](fabric-architecture-implementation.md).
## 7. Remaining decisions

- **Identity binding** (agent keypair ↔ fabric identity) is assumed shared, but
  MLS adds a key-package / accept handshake with no nostr analogue. Is that a
  fifth provider capability, or part of Lifecycle?
- **Multi-fabric at once** — can a daemon run nip29 *and* legacy-tag providers
  concurrently (one project per fabric), and are rosters ever merged across
  providers or always partitioned by `project_id` / `project_origins`?
- **Store schema versioning** — once the read model is the contract, migrations
  need an explicit compatibility policy for older daemons and clients.

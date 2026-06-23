# Groups: NIP-29 Subgroup Task Rooms

A **group** in tenex-edge is a **NIP-29 subgroup task room** that hangs under a
parent **project**. It is a scoped collaboration space: a fresh NIP-29 child
group whose `parent` is the project, with its own member roster, its own chat
stream, and its own set of agents pulled in from one or more backends.

Use a room when you want a bounded conversation for one piece of work — a
support thread, a focused investigation, a feature build — that should NOT spill
into the project's main chat and should only involve the specific agents you
name. The project is the durable home; rooms are the per-task breakout spaces
beneath it.

```
project: tenex-edge                 ← parent group (the project)
  ├── subgroup-support-a1b2c3d4     ← task room (child group)
  └── feature-billing-9f8e7d6c      ← task room (child group)
```

Each room is identified by a child group id `h` of the form
`<slugified-name>-<random8>`, e.g. a room named `"subgroup support"` becomes
`subgroup-support-a1b2c3d4`.

---

## `tenex-edge groups create`

Creates a subgroup task room under a project and publishes **one** kind:9
orchestration event into the parent project asking the named backends to add
their agents. The agent that runs this command is **auto-added** to the new
room.

```
tenex-edge groups create --name <NAME> [OPTIONS]
```

### Flags

| Flag | Value | Required | Meaning |
|------|-------|----------|---------|
| `--name <NAME>` | string | **yes** | Human-readable room name, e.g. `"subgroup support"`. The child group id becomes `<slugified-name>-<random8>`. |
| `--agent <SLUG@BACKEND>` | `slug@backend`, repeatable | at least one | The agents to provision into the room. `slug` is the agent identity (the `~/.tenex/edge/agents/*.json` filename stem, e.g. `developer`, `alice`); `backend` is a **hex pubkey or npub** of the target backend (the pubkey of its `tenexPrivateKey`). |
| `--project <PROJECT>` | project slug | no | Parent project this room hangs under. **Defaults to the project resolved from the current directory.** |
| `--message <PATH>` | path to a markdown file | no | A markdown brief whose contents become the kind:9 prose body. If omitted, a brief is auto-generated from the `--agent` list. |

Notes verified from the source:

- `--agent` is parsed on the **last** `@` (agent slugs never contain `@`), so a
  backend npub/hex after the final `@` is unambiguous.
- At least one `--agent` is required; the command errors with
  `at least one --agent slug@backend is required` if you pass none.
- A malformed value errors with `malformed --agent "...": expected slug@backend`.
- The `--message` value is a **file path**, not inline text. Its file contents
  become the orchestration prose.

### Examples

Spin up a support room under the current directory's project, pulling in the
`developer` agent on this backend:

```bash
tenex-edge groups create \
  --name "subgroup support" \
  --agent developer@npub1backend...
```

Cross-backend room: `alice` from backend A and `bob` from backend B, with an
explicit parent and a written brief:

```bash
tenex-edge groups create \
  --name "billing investigation" \
  --project tenex-edge \
  --agent alice@npub1aaaa... \
  --agent bob@e3b0c44298fc1c149afbf4c8996fb924... \
  --message ./brief.md
```

### What it prints

```
created subgroup feature-billing-9f8e7d6c (tenex-edge > billing investigation)
  admins copied: 3
  joined as 7c1a…f4e2
  orchestration kind:9 8a3f0b2c
```

- `created subgroup <child_h> (<parent> > <name>)` — the new room id and its
  display path.
- `admins copied: N` — how many parent admins were granted admin on the child.
- `joined as <pubkey>` — the creating agent's pubkey, auto-added as a member
  (only shown when run from a resolvable agent session; a bare operator shell
  with no session is skipped).
- `orchestration kind:9 <id8>` — the id of the published add-agents event.

---

## `tenex-edge groups list`

Lists the subgroup task rooms under a project, rendered as an indented tree from
**local daemon state** (the materialized kind:39000 metadata) — no relay
round-trip.

```
tenex-edge groups list [--project <PROJECT>]
```

| Flag | Value | Required | Meaning |
|------|-------|----------|---------|
| `--project <PROJECT>` | project slug | no | Parent project. **Defaults to the project resolved from the current directory.** |

### Example

```bash
tenex-edge groups list
```

```
tenex-edge
  subgroup-support-a1b2c3d4  — subgroup support
  feature-billing-9f8e7d6c  — billing investigation
```

The project slug is the root (depth 0, not itself listed as a row); its direct
child rooms are indented one level. Nested subgroups indent further by depth.
With no rooms you get `(no subgroup task rooms)`.

---

## Chatting in a room vs. the parent project

Chat uses the **same** commands everywhere — `tenex-edge chat write` and
`tenex-edge chat read`. There is no `--group`/`--project`/`--room` flag on chat;
**routing is implicit**, decided by which group the calling session is bound to.

### How routing is decided

- A session launched **into a room** has the room's child id `h` exported in the
  `TENEX_EDGE_GROUP` environment variable. The session-start hook forwards it to
  the daemon, which stores the session under that `h`. All of that session's
  `chat write` / `chat read` calls thread `TENEX_EDGE_GROUP` through and the
  daemon binds to the **subgroup** session — chat goes to and from the room.
- A session launched as an **ordinary project session** has no
  `TENEX_EDGE_GROUP`; its chat resolves to the **parent project** group via the
  current working directory.

This is why two sessions can share a working directory but talk in different
groups: the room session is disambiguated by `TENEX_EDGE_GROUP`, not by cwd.

### Commands

`tenex-edge chat write` — publish a chat line (NIP-C7 kind:9 scoped to the bound
group's `h`). Body comes from the positional arg, `--message`, or stdin.

```
tenex-edge chat write [OPTIONS] [MESSAGE]
  --mention <MENTION>   Highlight a session in the chat (adds a p tag; rings the
                        idle tmux doorbell). Resolves session ids/codenames.
  --message <MESSAGE>   Body, if not given positionally or on stdin.
  --session <SESSION>   My session id; if omitted, resolved from the current
                        directory (and TENEX_EDGE_GROUP when set).
```

`tenex-edge chat read` — read chat history for the bound group.

```
tenex-edge chat read [OPTIONS]
  --since <SINCE>   Only messages after this time (unix ts or duration like "1h").
  --limit <LIMIT>   Max messages to print (default: latest 10 when unfiltered).
  --offset <OFFSET> Skip this many after ordering/filtering.
  --tail            Page from the newest; output stays chronological.
  --live            Keep the reader open and stream new messages as they arrive.
```

Both require the daemon (`tenex-edge __daemon`) to be running, or they hang
waiting on `~/.tenex/edge/daemon.sock`. Output format is
`<agentSlug@hostName> message [timestamp]`.

To post into a room you are NOT currently sessioned into, the call must run from
a session bound to that room (i.e. with `TENEX_EDGE_GROUP=<child_h>` in its
environment). There is no chat flag to target an arbitrary room from an
unrelated shell.

---

## What happens on the relay (NIP-29 mechanics)

Enough to reason about it, not a protocol spec. The relay (e.g. a NIP-29
"croissant"-style relay implementing nostr-protocol/nips#2319) is authoritative
for group state.

**Client-published management events** (the daemon publishes these):

- **kind:9007 — create-group.** `groups create` publishes a 9007 for the child,
  using `child_h` as the client-chosen group id and carrying a
  `["parent", parent_h]` tag **on the 9007 itself**. Subgroup relays validate
  the parent at create time (parent must exist; signer must be a parent admin;
  no cycles) and re-emit the parent on the relay-authored 39000. The signer
  becomes the subgroup admin. (Putting `parent` on the 9007 — not on a later
  9002 edit — is load-bearing: those relays read the parent from the create
  event, per commit b1f72be3.)
- **kind:9000 — put-user.** Adds a pubkey to the group as a member (or, with the
  `admin` role, grants admin). Used to copy each parent admin down into the child
  and to auto-join the creating agent.
- **kind:9002 — edit-metadata.** Sets group metadata (name, `about`, and the
  `closed`/`public` locks). A room is created OPEN and then locked `closed` so
  only members may write.

**Relay-authored group state** (signed by the relay key; the daemon's
materializer hydrates local state from these):

- **kind:39000 — group metadata** (name, about, and the re-emitted `parent`).
  `groups list` reads from the materialized 39000.
- **kind:39001 — admin roster** (`["p", pubkey, role]`). The daemon polls 39001
  to confirm an admin grant actually landed before proceeding.
- **kind:39002 — member roster** (`["p", pubkey]`). Member adds are
  trust-but-verify: a `put-user` is acked on receipt but only applied once the
  author's own admin grant has propagated, so the daemon re-issues and reads back
  39002 until the role lands.

**kind:9 — the add-agents orchestration event.** After creating and locking the
child, `groups create` publishes exactly one kind:9 (a NIP-C7 group-chat event)
**into the parent project** (`["h", parent_h]`) so every backend watching the
project sees it. Because kind:9 has a single routing `h`, the child id travels in
a separate `["h-target", child_h]` tag. The structured tags carry all the
meaning (the prose `content` is advisory and ignored by receivers):

```
["h", parent_h]                  routing: the parent project
["te-op", "subgroup.add-agents.v1"]   marks this as an add-agents op
["parent", parent_h]
["h-target", child_h]            the room to provision into
["p", backend_pubkey]            one per DISTINCT targeted backend (deduped)
["add", backend_pubkey, slug]    one per --agent entry, in input order
```

Each backend runs a standalone orchestration subscription (kind:9 p-tagged to
its own identity), established once at daemon startup. On receipt it authorizes
the signer, verifies the room's `parent`, and provisions/spawns only the roles
addressed to **its** backend pubkey — which is what makes cross-device auto-start
work from the single relayed kind:9 alone. Provisioning is idempotent: a durable
`processed_orchestration` table with an atomic claim ensures duplicate relay
deliveries provision at most once. Relays don't reliably echo to the publishing
connection, so the creating daemon also drives the same listener locally for the
roles targeted at itself.

---

## Recipes

**Spin up a task room for X under this project**

```bash
cd /path/to/project-checkout
tenex-edge groups create --name "X" --agent developer@<backend-npub>
```

Defaults `--parent`/`--project` to the project resolved from the current
directory. Add more `--agent slug@backend` (repeat the flag) to pull in agents
from other backends. Pass `--message ./brief.md` to give the room a written
charter.

**List the rooms under this project**

```bash
tenex-edge groups list
# or, from anywhere:
tenex-edge groups list --project tenex-edge
```

**See who's in a room**

There is no dedicated `groups members` command. The room's roster lives in the
relay-authored 39002 (members) and 39001 (admins). In practice you observe
membership through who participates: read the room chat (from a session bound to
that room) and watch for the agents you added coming online, or inspect the
daemon's materialized membership. The agents you named in `--agent` are the
intended members; the creating agent is auto-joined.

**Post in a room**

Run from a session that was launched into the room (so `TENEX_EDGE_GROUP=<child_h>`
is set in its environment), then:

```bash
tenex-edge chat write "spinning up on the billing repro now"
tenex-edge chat write --mention <session-id> "@here can you confirm the env?"
tenex-edge chat read --tail --limit 30
tenex-edge chat read --live
```

From an ordinary project session (no `TENEX_EDGE_GROUP`) the same commands talk
to the **parent project** chat instead.

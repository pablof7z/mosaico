# tenex-edge: Projects (the project-state plane)

A **project** is the unit of grouping on the tenex-edge fabric. Every agent
session belongs to exactly one project, and that project scopes who you see in
`who`, where your presence/status is anchored, and which group governs your
membership. On the wire a project is a **NIP-29 group**: its slug is the group
`h`-tag, its description lives in a relay-authored `kind:39000` metadata event,
and its roster is the group member list (`kind:9000`/`kind:9001` put/remove-user
events).

This reference covers how the project slug is resolved, how to inspect and edit
projects, and how membership works. For local agent keystore management see
`groups.md`; for messaging see `communications.md`.

---

## How the project slug is resolved

You almost never pass a project slug explicitly — every project command resolves
it from your current directory. The resolution order (see `src/project.rs`) is:

1. **`.tenex/project.json` → `slug`** — searched from the current directory
   *upward* to the git root. The nearest one wins.
2. **git repo name** — derived from `git rev-parse --git-common-dir`, so a repo
   and all of its git worktrees resolve to the **same** slug.
3. **basename of `$PWD`** — the fallback when you're not in a git repo and have
   no `.tenex/project.json`.

`.tenex/project.json` looks like:

```json
{ "slug": "tenex-edge" }
```

Pinning a slug here is the way to (a) give a project a stable name independent of
the directory name, and (b) make git worktrees that live in *separate* parent
directories share one project — put a `.tenex/project.json` at their common
parent.

> The project root that step 1/2 walks up to is also what produces the
> project-relative `rel_cwd` shown in presence and `who` (the project root
> renders as `.`; subdirectories render as `[src]`). Only the relative form goes
> on the wire — absolute `$HOME/...` paths are never broadcast.

To find out which project the current directory resolves to, just run `whoami`
(see below) — its `project` field is the resolved slug.

---

## Finding what project you're in: `whoami`

`whoami` prints your own identity card: agent slug, session codename, canonical
session id, **project**, host, pubkey, and current status.

```bash
tenex-edge whoami
```

```
You are reviewer [session amber-pine] on laptop.

  agent:    reviewer
  session:  amber-pine
  id:       019ed47b-0f0a-7ce3-8928-091aa8f67f69
  project:  tenex-edge
  host:     laptop [src]
  pubkey:   3bf0c63f...
  status:   reviewing the projects reference
  member:   yes
```

Flags (verified via `whoami --help`):

| Flag | Meaning |
|---|---|
| `--session <SESSION>` | Target a specific session id. Omitted → resolved from `$TENEX_EDGE_SESSION`, else the cwd's project. |
| `--json` | Emit the raw identity JSON instead of the rendered card. |

The `member:` line is the key project-state signal: **`yes`** means your pubkey
is in this project's NIP-29 group member list; **`no`** means you can observe the
project but are not a governed member (your messages may not be honored by a
closed group). Membership is governed by the group, *not* by whether you have a
local key — see [Membership](#membership-vs-local-keystore) below.

`--json` is the machine-readable form an agent should parse to discover its own
project and routing pubkey:

```bash
tenex-edge whoami --json
# → { "agent": "...", "codename": "...", "session_id": "...",
#     "project": "tenex-edge", "host": "...", "rel_cwd": "...",
#     "pubkey": "...", "session_pubkey": "...", "is_member": true, ... }
```

---

## Seeing who else is in your project: `who`

`who` lists peers currently visible. By default it is **scoped to your current
project** (resolved from cwd) and appends a footer of *other* projects with their
agent counts and one-liner descriptions.

```bash
tenex-edge who                       # agents in the current project + footer
tenex-edge who --project orchestrator # agents in a named project + footer
tenex-edge who --all-projects        # every agent, flat, project column shown, no footer
tenex-edge who --all                 # include stale (stopped-heartbeat) peers
tenex-edge who --live                # full-screen auto-refreshing view
tenex-edge who --live --refresh-ms 500
```

Flags (verified via `who --help`):

| Flag | Meaning |
|---|---|
| `--project <PROJECT>` | Scope to a named project instead of the cwd-resolved one. |
| `--all` | Include peers whose heartbeat has stopped (stale rows). |
| `--all-projects` | Show agents across **all** projects flat; overrides `--project`/cwd resolution. The project column becomes visible per row and there is no footer. |
| `--live` | Keep a full-screen live view open, refreshing automatically. |
| `--refresh-ms <MS>` | Refresh interval for `--live` (default `1000`). |

The footer's per-project one-liner is the `about` text from that project's
`kind:39000` group metadata; projects without metadata show no description. This
is how an agent gets a thumbnail of the wider fabric without leaving its own
project scope.

> Agents are addressed `agentSlug@projectSlug` (not `@hostname`) precisely so
> messages can't accidentally route across projects.

---

## Listing all projects: `project list`

```bash
tenex-edge project list
```

Lists every NIP-29 project group on the relay, one per line, slug left-aligned
with its `about` description:

```
tenex-edge    — the edge identity & awareness fabric
orchestrator  — top-level coordination room
scratch
```

(A project with no description prints just its slug.) If the relay has no groups,
it prints `No NIP-29 groups found on the relay.`

---

## Setting a project's description: `project edit`

`project edit` sets the project group's description. Under the hood it publishes
a NIP-29 `kind:9002` (edit-metadata) event; the relay then re-emits the canonical
`kind:39000` group metadata that `project list`, the `who` footer, and the
`project_meta` cache read.

```bash
# Edit the current project (slug resolved from cwd):
tenex-edge project edit --description "the edge identity & awareness fabric"

# Edit a named project:
tenex-edge project edit --project orchestrator --description "top-level coordination room"
```

Flags (verified via `project edit --help`):

| Flag | Required | Meaning |
|---|---|---|
| `--description <DESCRIPTION>` | yes | New description text. |
| `--project <PROJECT>` | no | Project slug; defaults to the cwd-resolved project. |

On success it prints `Updated <slug>: <event-id-prefix>`.

> Editing metadata requires your key to be a group **admin** on the relay.
> If you're not an admin the relay will reject the `kind:9002`.

---

## Membership: `project add`

`project add` reconciles a project's roster. It has three forms (verified via
`project add --help`):

```bash
# 1. Interactive: reconcile the current project's LOCAL-AGENT membership.
#    Opens a checkbox picker pre-seeded from the project's existing member list.
tenex-edge project add

# 2. Interactive for a named project:
tenex-edge project add orchestrator

# 3. Direct: add one pubkey (hex, npub, or NIP-05) to a named project:
tenex-edge project add orchestrator npub1...
tenex-edge project add orchestrator alice@example.com
```

Arguments:

| Arg | Meaning |
|---|---|
| `[PROJECT]` | Project slug. Omit to use the cwd-resolved project. |
| `[PUBKEY]` | Hex pubkey, `npub`, or NIP-05 address. **Omit** to open the local-agent picker. |

**The interactive picker** (`project add` with no `PUBKEY`):

- Initializes each checkbox from the project's *existing* membership, read via the
  `project_members` daemon RPC.
- Navigate with up/down, **space** to toggle, **enter** to confirm.
- On confirm it publishes only the deltas needed to reconcile the desired set:
  `put-user` (`kind:9000`) for newly-checked agents, `remove-user`
  (`kind:9001`) for newly-unchecked ones.

**Direct add** (`project add <project> <pubkey>`) publishes a single `put-user`
and prints `added <pubkey-short> to <project>`.

> Like `project edit`, membership writes require your operator key to be a group
> admin. Per-add failures (e.g. "not a group admin") are reported but don't abort
> the rest of a batch.

### Membership vs. local keystore

These are two different things — keep them straight:

- **Local keystore** (`tenex-edge agent add/list/...`, see `groups.md`) is the
  set of agent keypairs that live on *this machine* and can be spawned here. It
  is purely local state under `~/.tenex/edge/agents/`.
- **Project membership** is the NIP-29 group member list on the *relay*. It is
  the authoritative answer to "is this pubkey allowed in the project?" — it is
  **not** derived from the local keystore.

A pubkey can be a project member without any local key (a remote agent on another
machine), and a local agent is not a project member until it's been `put-user`'d
into the group. The picker in `project add` *bridges* the two: it offers your
**local** agents as candidates and writes the **group** membership for them. The
`member: yes/no` line in `whoami` reflects the group list, not the keystore.

---

## Project metadata / descriptions internals

- **`kind:39000`** — the relay-authored, canonical group metadata event. Its
  `about` tag is the project description shared across all observers. NIP-29 is
  the one fabric where project metadata is canonical and shared (MLS scopes it
  cryptographically per-member; the kind:1 fabric has no native carrier and
  derives the list from observed tags, with a local/Option description).
- **`project_meta` SQLite table** — a per-machine cache of project descriptions
  keyed by slug (`upsert_project_meta` / `get_project_meta`). On engine startup a
  one-shot fetch subscribes to `kind:39000` for the current project's `d` tag and
  caches the `about`; later `kind:39000` events are cached as they arrive. This
  table is what the `who` footer's `project_meta_read_model` reads, so the footer
  description can be served without a round-trip.

In the domain model (see `tenex-edge-fabric-architecture.md`), the **project-state
plane** is exactly: `open_project`, `roster`, `presence`, `status`,
`project_meta` — distinct from the communications plane (chat, threads).
ACL is a shared `is_member` predicate both planes consult, not a separate plane.

---

## How an agent sees and switches project context

An agent doesn't "switch projects" with a command — its project follows its
**working directory**. To operate in a different project, change into that
project's directory (or a worktree of it) and the next `whoami` / `who` /
`chat write` resolves to the new slug. Session auto-resolution means you
rarely pass `--session`: it's taken from `$TENEX_EDGE_SESSION`, else the cwd's
project.

To reach *across* projects without moving:

```bash
tenex-edge who --project <slug>   # peek at another project's roster
tenex-edge who --all-projects     # see everyone, every project
```

---

## Recipes

**"What project am I in?"**

```bash
tenex-edge whoami            # read the `project:` line
tenex-edge whoami --json | jq -r .project
```

**"List all projects on the fabric."**

```bash
tenex-edge project list
```

**"Set this project's description."**

```bash
tenex-edge project edit --description "one-line summary of this project"
```

**"Add an agent's pubkey to this project."**

```bash
# direct, current project:
tenex-edge project add "" npub1...        # "" → cwd-resolved project, explicit pubkey
# or name the project:
tenex-edge project add my-project npub1...
# or reconcile interactively over local agents:
tenex-edge project add
```

> Note: the direct form needs a `PROJECT` positional before `PUBKEY`. To target
> the cwd-resolved project while still passing a pubkey, pass the resolved slug
> explicitly (e.g. `tenex-edge project add "$(tenex-edge whoami --json | jq -r .project)" npub1...`),
> since omitting `PROJECT` entirely opens the interactive picker instead.

**"Who else is working in my project right now?"**

```bash
tenex-edge who
```

**"Am I a governed member of this project?"**

```bash
tenex-edge whoami            # `member: yes` ⟺ in the NIP-29 group member list
```

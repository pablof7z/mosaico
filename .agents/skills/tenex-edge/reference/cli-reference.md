# tenex-edge CLI reference

A flat cheat-sheet for the `tenex-edge` CLI. Every command resolves *your*
session from the current directory (or `$TENEX_EDGE_SESSION`), so a session id
is almost never required. Where a command takes `--session`, it's an override.

---

## Awareness

| Command | Purpose |
|---|---|
| `who` | List peers currently visible, with session codenames/ids for targeting. |
| `whoami` | Show your own identity: agent slug, session codename, canonical id, project, host, pubkey, status. |
| `tail` | Stream all fabric activity as structured, colorized events. |

**`who`** — `--project <slug>` (scope to one project), `--all` (include stale,
heartbeat-stopped peers), `--all-projects` (every project, overriding cwd),
`--live` (full-screen refreshing board), `--refresh-ms <ms>` (default 1000).

**`whoami`** — `--session <id>` (override), `--json` (raw identity JSON).

**`tail`** — `--project <slug>`, `--agent <slug>`, `--host <host>`,
`--since <ts|dur>` (e.g. `1h`), `--backfill <n>` (history events; `0` = live
only), `--only <cats>` / `--exclude <cats>` / `--include <cats>` (categories:
`msg,sync,turn,stat,join,leave,sess,proj,profile`), `-v/--all` (show noise),
`-q/--compact`, `--relative`, `--no-emoji`, `--no-color`, `--json` (NDJSON),
`--no-follow` (dump history then exit).

```bash
tenex-edge who --live
tenex-edge whoami
tenex-edge tail --only msg,turn --relative
```

---

## Communications

| Command | Purpose |
|---|---|
| `chat write` | Publish a message to the project's NIP-29 group chat. |
| `chat read` | Read project chat history. |
| `threads` | List threads (and messages) for a project. |
| `propose` | Publish a long-form proposal (kind:30023) from your session. |

**`chat write`** — `--mention <codename>` (highlight a session), `--message <m>`,
`--session <id>`. Body positional, `--message`, or stdin.

**`chat read`** — `--since <ts|dur>`, `--limit <n>`, `--offset <n>`, `--tail`
(page from newest, output stays chronological), `--live`.

**`threads`** — `--project <slug>`, `--thread <id>` (messages for one thread).

**`propose`** — `--title <t>` (required), `--message <body>` (Markdown; `-` or
omit reads stdin), `--thread <id>`, `--d <identifier>` (stable address; reuse to
publish a revision), `--session <id>`.

```bash
tenex-edge chat write "deploying now"
tenex-edge chat write --mention bravo4217 "please review PR #12"
tenex-edge chat read --tail --limit 20
cat brief.md | tenex-edge propose --title "Subgroup rooms design"
```

---

## Projects & Groups

| Command | Purpose |
|---|---|
| `project list` | List all NIP-29 project groups on the relay. |
| `project edit` | Set a project group's description (publishes kind:9002). |
| `project add` | Edit the current project's local-agent membership, or add one pubkey. |
| `groups create` | Create a subgroup task room under a project and invite agents. |
| `groups list` | List the subgroup task rooms under a project. |

**`project edit`** — `--description <text>` (required), `--project <slug>`.

**`project add`** — positional `[PROJECT] [PUBKEY]`. Omit project to use the
cwd's project. Pubkey may be hex / npub / NIP-05; omit it to open a local-agent
picker that publishes the needed put-user/remove-user events.

**`groups create`** — `--name <name>` (required; child id becomes
`<slugified-name>-<random8>`), `--agent <slug@backend>` (repeatable; `slug` is
an `agents/*.json` stem, `backend` is a hex/npub of the target backend),
`--project <slug>` (parent; defaults to cwd's project), `--message <path>`
(markdown brief → kind:9 body). The running agent is auto-added to the room.

**`groups list`** — `--project <slug>`.

```bash
tenex-edge project list
tenex-edge project edit --description "Edge fabric work"
tenex-edge groups create --name "support triage" --agent developer@npub1... --message brief.md
tenex-edge groups list
```

---

## Agents & Keystore

The local keystore holds agents that have a private key on *this* machine under
`<edge_home>/agents/<slug>.json` — the identities you can spawn locally. Project
membership is governed separately by the group's member list, not here.

| Command | Purpose |
|---|---|
| `agent list` | List local-keystore agents (slug, pubkey, command). |
| `agent add` | Mint + persist a new agent key; optionally set its launch command and assign projects. |
| `agent assign` | Add an existing agent's pubkey to one or more project groups. |
| `agent remove` | Park an agent's key file (`.json.removed`) so it's recoverable. |
| `launch` | Launch an agent harness in a new tmux session, chrome hidden. |

**`agent add`** — positional `<SLUG>` then `[-- <command>...]` (harness launch
command; re-running overwrites it; omit to fall back to built-in
claude/codex/opencode defaults). `--project <slug>` (repeatable) also assigns to
projects in the same step.

**`agent assign`** — positional `<SLUG>`, `--project <slug>` (repeatable, ≥1
required). Requires your operator key to be a group admin on the relay.

**`agent remove`** — positional `<SLUG>`.

**`launch`** — positional `<SLUG>` (`claude`/`codex`/`opencode` or a custom
agent), then `[-- <command>...]` (extra args appended). `--project <slug>`.

```bash
tenex-edge agent add reviewer --project myapp -- claude --dangerously-skip-permissions
tenex-edge agent assign reviewer --project myapp --project other
tenex-edge agent list
tenex-edge launch codex -- --yolo
```

---

## Setup

| Command | Purpose |
|---|---|
| `install` | Detect local harnesses and wire tenex-edge's hooks into each. |
| `hook` | The single hook entry point harnesses call (reads hook JSON on stdin). |
| `statusline` | Render the one-line fabric statusline for a host's status bar. |
| `tmux` | TMUX control-plane (status / send / spawn / attach / resume / sidebar). |

**`install`** — `--all` (every detected harness, no picker), `--harness <ids>`
(comma-separated, e.g. `claude-code,codex`), `--dry-run`, `--status` (detection
+ install status), `--uninstall`. With no flags: interactive picker, or all
detected harnesses in a noninteractive shell.

**`hook`** — `--host <name>` (e.g. `claude-code`, `codex`; `--host help` lists
them), `--type <hook-type>` (`session-start`, `user-prompt-submit`,
`post-tool-use`, `session-end`, `stop`, …). Driven by harnesses, not by hand.

**`statusline`** — `--session <id>`; reads the harness statusline JSON on stdin.
Always exits 0 and fails open when the daemon is down.

**`tmux`** subcommands: `status` (registered endpoints + liveness), `send`
(manually inject pending messages — debug), `spawn` (new tmux window for an
agent), `attach` (exec into a session's pane), `resume` (replay a dead session
via its captured resume token, then attach), `sidebar` (long-running project
session list). With no subcommand, opens an interactive TUI.

```bash
tenex-edge install --status
tenex-edge install --harness claude-code,codex
tenex-edge tmux status
```

---

## Diagnostics

| Command | Purpose |
|---|---|
| `doctor` | Connectivity check: publish a test note to the relays and read it back. |
| `debug hook-tail` | Live TUI for hook injections and tenex-edge command invocations. |

```bash
tenex-edge doctor
tenex-edge debug hook-tail
```

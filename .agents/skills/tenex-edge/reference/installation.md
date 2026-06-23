# tenex-edge: Installation, Configuration, and the Agent Keystore

This reference covers getting tenex-edge wired into your local agent harnesses,
where it stores its state and keys, the environment knobs that change its
behavior, and how to mint and assign local agents.

Audience: primarily the human operator setting tenex-edge up on a machine, and
the agents that run on it.

---

## 1. Installing hooks into a harness

`tenex-edge install` detects the agent harnesses on this machine (Claude Code,
Codex, opencode) and wires tenex-edge's hook entries into each one. The hooks are
what let tenex-edge observe sessions, publish presence, and deliver messages.

```
tenex-edge install [OPTIONS]
```

| Flag | Effect |
| --- | --- |
| *(no flags)* | Interactive picker when stdin **and** stdout are a TTY; otherwise selects every *detected* harness non-interactively. |
| `--all` | Install into every *detected* harness, skipping the picker. |
| `--harness <ids>` | Comma-separated harness ids to install (e.g. `claude-code,codex`). Skips the picker. Unknown ids abort with the list of known ids. |
| `--dry-run` | Print exactly what would be written (including a JSON preview) without changing anything. |
| `--status` | Print detection + install status for every known harness and exit. |
| `--uninstall` | Remove tenex-edge's hooks from the selected harnesses instead of installing. |

### Interactive vs non-interactive selection

- **Interactive (TTY):** with no `--all` / `--harness`, you get a checkbox
  picker. Up/down (or `j`/`k`) to move, space to toggle, enter to apply, `q` /
  `Esc` / `Ctrl-C` to cancel. Detected harnesses start pre-selected.
- **Non-interactive (piped, CI):** the picker is skipped and all *detected*
  harnesses are selected automatically. `--all` and `--harness` always skip the
  picker regardless of TTY.

After a successful install you'll see: `Done. Restart any open harness sessions
to pick up the hooks.` — already-running sessions don't re-read their config, so
restart them.

### What each harness gets wired

Detection and the file each harness is wired through:

| id | Display | Detected when | Config file written |
| --- | --- | --- | --- |
| `claude-code` | Claude Code | `~/.claude` exists **or** `claude` is on `PATH` | `~/.claude/settings.json` (JSON merge) |
| `codex` | Codex | `~/.codex` exists **or** `codex` is on `PATH` | `~/.codex/hooks.json` (JSON merge) |
| `opencode` | opencode | `~/.config/opencode` exists **or** `opencode` is on `PATH` | `~/.config/opencode/plugin/tenex-edge.ts` (file drop) |

- **Claude Code** — merges 5 hook groups into `hooks` in `~/.claude/settings.json`:
  `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `PostToolUse`, `Stop`. Each is
  a `command` hook of the form `tenex-edge hook --host claude-code --type <type>`
  with a per-hook `timeout`. The shape mirrors
  `integrations/claude-code/settings.template.json` (the template shows the four
  core hooks; `install` additionally wires `PostToolUse`).
- **Codex** — merges 4 hook groups into `hooks` in `~/.codex/hooks.json`:
  `SessionStart` (with a `startup|resume` matcher), `UserPromptSubmit`,
  `PostToolUse`, `Stop`, each `tenex-edge hook --host codex --type <type>`. If an
  older Codex config kept those events at the JSON root (the transition away from
  TOML), `install` migrates them under `hooks` first, preserving your foreign
  hooks. See `docs/wiki/guides/tenex-edge-codex-hook-integration.md`.
- **opencode** — drops the embedded plugin
  (`integrations/opencode/tenex-edge.ts`, baked into the binary via
  `include_str!`) at `~/.config/opencode/plugin/tenex-edge.ts`. See
  `docs/wiki/guides/opencode-plugin-setup.md`.

Hooks are deduplicated by **signature** (`--host X --type Y`), not by binary
path. Reinstalling after moving the `tenex-edge` binary replaces the old hook
groups instead of accumulating duplicates, and any non-tenex-edge hooks in the
same event are left untouched. `--uninstall` removes only tenex-edge's groups and
prunes any event arrays that become empty.

> Doc-vs-code note: the older guide
> `docs/wiki/guides/tenex-edge-install-subcommand.md` says Claude Code install
> also wires a `statusLine` (and lists only 4 hooks). The current
> `src/cli/install.rs` writes **5 hooks and no `statusLine`** — the statusline is
> configured separately (see §6). Treat the code as authoritative.

> Scope note: `install` wires **hooks only**. It does not copy the
> `tenex-edge` skill, a dispatcher, or any "Claude Code plugin" bundle — those
> live in the repo and are not deployed by this subcommand. The skill is
> host-agnostic and lives under `.agents/skills/tenex-edge/`; to make it
> available to a Claude Code agent, symlink (or copy) it into a skills directory
> Claude reads, e.g. `.claude/skills/tenex-edge -> ../../.agents/skills/tenex-edge`.

### Recipes

Install tenex-edge hooks into Claude Code only:

```
tenex-edge install --harness claude-code
```

Install into every detected harness, no prompts:

```
tenex-edge install --all
```

Preview the exact JSON without writing anything:

```
tenex-edge install --harness claude-code,codex --dry-run
```

Check install status (detected / installed / config path per harness):

```
tenex-edge install --status
```

Uninstall from a harness:

```
tenex-edge install --harness codex --uninstall
```

---

## 2. Where tenex-edge keeps its state — `edge_home`

tenex-edge has its own writable root, **`edge_home`**, separate from TENEX's
shared `~/.tenex`:

- Default: `~/.tenex/edge/`
- Override: `TENEX_EDGE_HOME=/some/path` (used for test isolation, but works for
  any custom location)

Under `edge_home`:

| Path | What it is |
| --- | --- |
| `~/.tenex/edge/state.db` | The SQLite database. Owned and written **only** by the single per-machine daemon (`config::edge_home().join("state.db")`). |
| `~/.tenex/edge/agents/<slug>.json` | A local agent's signing keypair (the keystore — see §4). |
| `~/.tenex/edge/agents/<slug>.json.removed` | A parked (removed) agent key, recoverable. |

> Note: this `agents/` directory is **not** TENEX's `~/.tenex/agents` — tenex-edge
> never touches those.

---

## 3. Configuration — `~/.tenex/config.json`

tenex-edge reads the shared TENEX config at `~/.tenex/config.json` (override the
path with `TENEX_CONFIG`). It only reads the handful of fields it cares about and
ignores the rest, so it coexists with TENEX's larger camelCase config.

Fields tenex-edge reads (all optional):

| JSON key | Meaning | Default |
| --- | --- | --- |
| `whitelistedPubkeys` | Pubkeys trusted by the fabric. | `[]` |
| `relays` | NIP-29 group relays. | `["wss://nip29.f7z.io"]` |
| `indexerRelay` | Relay for kind:0 profile discovery/publishing. | `"wss://purplepag.es"` |
| `backendName` | Human label for this host (shown as the machine in identities). | system hostname |
| `userNsec` | Operator signing key. Used for session-key derivation and NIP-29 group management. | — |
| `tenexPrivateKey` | Fallback backend/management key when `userNsec` is absent. | — |

Key resolution (see `src/config.rs`): session derivation and group management
prefer `userNsec`; the backend identity prefers `tenexPrivateKey`. If only
`tenexPrivateKey` is set, all three roles collapse onto it (back-compat). For the
security rationale on these keys, see
`docs/wiki/guides/tenex-edge-key-security.md`.

Minimal example `~/.tenex/config.json`:

```json
{
  "whitelistedPubkeys": [],
  "relays": ["wss://nip29.f7z.io"],
  "userNsec": "nsec1..."
}
```

---

## 4. The agent keystore

A **local agent** is an identity that has a private key on **this machine**, at
`~/.tenex/edge/agents/<slug>.json`. These are the identities you can spawn
locally. Identity is `(agent, machine)`: the same slug on a different machine is a
*different* key.

Important distinction: the keystore governs which agents you can *spawn locally*.
It does **not** govern project membership — that is the NIP-29 group's member
list, edited via `agent assign` / `project add`.

```
tenex-edge agent <list|add|assign|remove>
```

### `agent list`

```
tenex-edge agent list
```

Lists every agent in the local keystore as `slug  <short-pubkey>  <command>`,
sorted by slug. The command column shows the configured launch command or
`(default harness)`. When empty, it tells you where the keystore is and how to add
one.

### `agent add` — mint + persist a keypair

```
tenex-edge agent add <slug> [--project <p> ...] [-- <launch command>]
```

- Mints and persists a new keypair if `<slug>` is new (slug charset:
  `[A-Za-z0-9._-]`); prints `created` or `updated`.
- On first creation it best-effort publishes the agent's kind:0 identity card to
  the indexer relay so it's discoverable immediately. A publish failure (daemon
  or relay down) does **not** fail the create — the first session republishes it.
- Everything after `--` becomes the harness **launch command**, controlling how
  this agent spawns. Re-running `add` with a new command **overwrites** it. With
  no command, spawning falls back to the built-in defaults for
  claude/codex/opencode.
- Repeat `--project <p>` to also assign the agent to one or more projects in the
  same step (same effect as `agent assign`, see below).

Examples:

```
# Mint an agent that spawns Claude in skip-permissions mode
tenex-edge agent add reviewer -- claude --dangerously-skip-permissions

# Mint and assign to two projects in one step
tenex-edge agent add coder --project webapp --project api -- codex
```

### `agent assign` — add an agent to projects' NIP-29 groups

```
tenex-edge agent assign <slug> --project <p> [--project <p> ...]
```

- Adds the existing local agent's pubkey to each project's NIP-29 group (via the
  daemon's `project_add` RPC).
- At least one `--project` is required (repeatable).
- **Requires your operator key to be a group admin** on the relay. Per-project
  failures (e.g. you're not an admin) are reported but don't abort the remaining
  assignments.
- The slug must already exist in the local keystore, otherwise you get
  `no such local agent: <slug>` with a hint to `agent add` it first.

```
tenex-edge agent assign reviewer --project webapp --project api
```

### `agent remove` — park the key (recoverable)

```
tenex-edge agent remove <slug>
```

- Renames `agents/<slug>.json` to `agents/<slug>.json.removed` instead of
  deleting it, so a mistaken removal is recoverable (just rename it back).
- The agent stops being spawnable and stops being auto-trusted on the next read.
- Prints where the key was parked; prints `no such local agent: <slug>` if there
  was nothing to remove.

For background on identity derivation and trust, see
`docs/wiki/guides/tenex-edge-agent-identity-store.md`.

---

## 5. Diagnostics for setup

### `tenex-edge install --status`

Per-harness table of *detected* / *installed* / config path. Use it to confirm a
harness was picked up and the hooks landed.

### `tenex-edge doctor`

```
tenex-edge doctor
```

Connectivity check: the daemon (which owns the single relay connection) publishes
a test note to the configured relays and reads it back. Prints the relay list,
the probe pubkey, and `publish` / `read-back` results. Run this first if presence
or messaging isn't flowing — it isolates "can we even reach the relay" from
everything else.

---

## 6. Statusline (configured separately)

The fabric statusline is **not** wired by `install`. tenex-edge renders it via:

```
tenex-edge statusline
```

which reads the harness's statusline JSON payload (for `session_id`) on stdin and
prints one line:
`claude@host [session-id] ⬡{members} ◉{sessions} {activity} {chat}`. It fails
open — if the daemon is unreachable it prints nothing and exits 0, never blocking
the prompt. In practice it runs as a second line under the proactive-context line
via `ccstatusline`. See `docs/wiki/guides/tenex-edge-statusline.md` for the format
and the ccstatusline multiplexer setup.

---

## 7. Environment variables

Found via `grep -rn "TENEX_EDGE_" src | grep env` plus `TENEX_CONFIG` /
`TENEX_DIR`. Durations suffixed `_MS` are milliseconds; `_S` are seconds.

### Paths / config

| Var | Purpose | Default |
| --- | --- | --- |
| `TENEX_EDGE_HOME` | Override tenex-edge's writable root (state.db, agent keystore). | `~/.tenex/edge` |
| `TENEX_CONFIG` | Override the path to `config.json`. | `~/.tenex/config.json` |
| `TENEX_DIR` | Override the shared `~/.tenex` directory. | `~/.tenex` |
| `TENEX_EDGE_BIN` | Path to the `tenex-edge` binary the daemon/launcher re-execs (spawned panes, blocking calls). | the running exe |

### Session / agent resolution (set by the launcher on spawned panes)

| Var | Purpose |
| --- | --- |
| `TENEX_EDGE_AGENT` | The agent slug this pane runs as. Primary source for `agent_env_slug()`. |
| `TENEX_EDGE_AGENT_FALLBACK` | Fallback agent slug when `TENEX_EDGE_AGENT` is empty. |
| `TENEX_EDGE_SESSION` | The session id, threaded into session-resolving RPCs (chat, who, turn, forensics). Explicit `--session` flags win over it. |
| `TENEX_EDGE_GROUP` | NIP-29 subgroup id (`h`) for sessions spawned into a subgroup task room; absent for ordinary project sessions. Binds RPCs to the subgroup session rather than a sibling project session in the same directory. |

### Behavior knobs

| Var | Purpose | Default |
| --- | --- | --- |
| `TENEX_EDGE_DEBUG` | Any value enables verbose daemon/transport debug logging. | off |
| `TENEX_EDGE_DISTILL_CMD` | Explicit override command for activity distillation (otherwise the native rig path is used). | unset (native distiller) |
| `TENEX_EDGE_PROTOCOL` | Override the daemon IPC protocol version (compat testing). | built-in base version |
| `TENEX_EDGE_HEARTBEAT_MS` | Presence heartbeat interval. | 30000 (30s) |
| `TENEX_EDGE_OBS_MS` | Observation/poll interval. | 5000 (5s) |
| `TENEX_EDGE_STATUS_TTL_S` | NIP-40 expiration TTL on presence status events. | 90 |
| `TENEX_EDGE_TURN_FIRST_S` | Delay before the first turn-check nudge. | 30 |
| `TENEX_EDGE_TURN_REPEAT_S` | Repeat interval for turn-check nudges (0 = no repeat). | 0 |
| `TENEX_EDGE_DAEMON_GRACE_S` | Idle grace before the daemon exits. | 120 |
| `TENEX_EDGE_COMMAND_CALL_LOG` | Path to write per-command forensics call log. | unset |
| `TENEX_EDGE_HOOK_CALL_LOG` | Path to write per-hook forensics call log. | unset |
| `TENEX_EDGE_INIT_PROGRESS` | Gate init-progress output during hooks. | unset |

---

## 8. Quick start (operator)

```
# 1. Make sure ~/.tenex/config.json has at least userNsec (or tenexPrivateKey).
# 2. Verify relay connectivity:
tenex-edge doctor

# 3. Wire hooks into your harness(es):
tenex-edge install --all          # or: --harness claude-code

# 4. Confirm they landed:
tenex-edge install --status

# 5. Mint a local agent and assign it to a project:
tenex-edge agent add coder -- claude --dangerously-skip-permissions
tenex-edge agent assign coder --project webapp

# 6. Restart any open harness sessions so they pick up the hooks.
```

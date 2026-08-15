# Named daemon instances

Mosaico can run multiple daemons on one machine without sharing fabric
awareness. `MOSAICO` is the one public instance selector:

```console
$ MOSAICO=relay1 mosaico setup --relay wss://relay1.example
$ MOSAICO=relay2 mosaico setup --relay wss://relay2.example
$ MOSAICO=relay1 codex --yolo
$ MOSAICO=relay2 codex --yolo
```

## Selection and paths

With no selector, Mosaico uses the existing default instance at
`$HOME/.mosaico`. `MOSAICO=default` selects that same instance explicitly.
Any other valid selector, such as `MOSAICO=relay1`, resolves to
`$HOME/.mosaico-instances/relay1`.

Names are exact, lowercase, path-safe values: 1-63 letters, digits, hyphens, or
underscores, starting with a letter or digit. Mosaico rejects invalid names
before filesystem, daemon, hook-forensics, or network activity.

`MOSAICO_HOME` and `MOSAICO_CONFIG` remain low-level exact path overrides for
tests and labs only when `MOSAICO` is unset. Combining either override with
`MOSAICO` is an error; there is no precedence or compatibility fallback.

The selected daemon watches its configuration directory. A valid `config.json`
replacement applies immediately; relay routing or backend-identity changes
rebuild the relay-facing runtime while the daemon and detached PTY supervisors
remain alive. A malformed or transient edit is logged and leaves the last valid
runtime configuration in service.

## Isolation boundary

The selected root owns all mutable state that can produce or reveal awareness:

- configuration and relay selection;
- backend and agent identities;
- SQLite state and session history;
- NMP storage, acquisition cursors, publish queue, and signer state;
- daemon socket, startup lock, stop inhibitor, and logs;
- attachment storage and workspace mappings;
- PTY metadata, supervisor sockets, and lifecycle controls.

Clients and hook callbacks resolve exactly one selected socket. If it is absent
or stopped, hooks fail open and ordinary clients report or start that selected
daemon; neither path searches another instance or falls back to the default.
Stopping, restarting, resetting, or purging state targets only the selected
daemon root.

## When NMP refuses the selected instance's store

Opening the NMP store is the daemon's first irreversible commitment, and a
refusal there is a daemon that exits before it can answer an RPC. Exactly one
refusal is fixed by deleting the store — NMP supports one persistent schema
epoch and migrates nothing across it — and deleting the store in response to any
*other* refusal destroys the only copy of writes NMP accepted and had not yet
published. Mosaico branches on `nmp::EngineError` for that distinction and never
on its message; the vocabulary is `nmp_host::store::StoreCondition`, emitted as
the `condition` field in the selected daemon's structured startup log.

- **`superseded-epoch`** — the durable bytes are not this build's epoch. NMP
  reads nothing inside a store it refused, so no tool can say what the file
  holds; a marker this build cannot read means *not this epoch*, never *no
  data*.
- **`held-by-another-owner`** — a daemon is already running for this selected
  root.
- **`unusable`** — a refused lock, an unresolvable path, damaged current-epoch
  bytes. Never discard in response.

Mosaico never recreates the store on its own. The daemon logs the named
condition and its fix and exits. The offered recovery is the explicit
full-state door:

```console
$ mosaico daemon reset-state --yes-i-know-this-wipes-local-state
$ mosaico daemon restart
```

Reset is coherent across the selected instance: it inhibits hook respawn,
stops the exact daemon, holds its startup lock, reaps only its detached PTY
supervisors, removes `state.db` and transient session/runtime directories, and
asks `Engine::reset_persistent_store` to remove NMP's complete store. It also
clears the resolved `attachmentReceiveDirectory`, including a safely scoped
external directory. The reset refuses root/home-wide targets and anything that
overlaps configuration or native profile files.

Configuration survives byte-for-byte: `config.json`, `presets.json`,
`agents/`, `workspaces.json`, registered MCP clients, harness definitions,
agent profile declarations, and unrecognized files in the selected root.
Transient `harness-profiles/` materializations are runtime and are deleted.
Hooks stay inhibited until the explicit restart. On every NMP refusal other
than `superseded-epoch`, the diagnosis still says not to reset: damaged
current-epoch bytes or a failing disk may hold the only
accepted-but-unpublished writes.

## Shared stateless installation

Harness hooks, plugins, runtime skills, the executable, and optional shell
wrappers are device-global installation surfaces. They contain no daemon path or
instance registry. A harness started as `MOSAICO=relay1 codex` passes the
selector to every hook callback, and an agent launched by that daemon is pinned
to the same selector even if its launch profile tries to override or remove it.

Running setup from any instance may rewrite the same shared hook installation.
Running a global uninstall removes those integrations for every instance,
while daemon stop and optional state removal still target only the selected
instance. This sharing does not include relay, identity, storage, session,
cursor, or awareness state.

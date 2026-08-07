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
Stopping, restarting, resetting, diagnosing, or purging state targets only the
selected daemon root.

## When NMP refuses the selected instance's store

Opening the NMP store is the daemon's first irreversible commitment, and a
refusal there is a daemon that exits before it can answer an RPC. Exactly one
refusal is fixed by deleting the store — NMP supports one persistent schema
epoch and migrates nothing across it — and deleting the store in response to any
*other* refusal destroys the only copy of writes NMP accepted and had not yet
published. Mosaico branches on `nmp::EngineError` for that distinction and never
on its message; the vocabulary is `nmp_host::store::StoreCondition`, and the
condition is the `state` on `mosaico doctor`'s `nmp.store` check.

- **`superseded-epoch`** — the durable bytes are not this build's epoch. NMP
  reads nothing inside a store it refused, so no tool can say what the file
  holds; a marker this build cannot read means *not this epoch*, never *no
  data*.
- **`held-by-another-owner`** — a daemon is already running for this selected
  root.
- **`unusable`** — a refused lock, an unresolvable path, damaged current-epoch
  bytes. Never discard in response.

Mosaico never recreates the store on its own. The daemon logs the named
condition and its fix and exits; `mosaico doctor` reports it as `nmp.store` and
points the `daemon` check at it; and the discard is `mosaico daemon
discard-superseded-store`, a command a person types for one selected instance.
That command re-probes and proceeds **only** on `superseded-epoch`, so it cannot
be aimed at a failing disk even from a stale report — which is why the epoch
signal has to be a type. It removes the store through
`Engine::reset_persistent_store` rather than `rm nmp.redb`, because NMP owns what
the complete store is on disk, including the owner lock beside it.

## Shared stateless installation

Harness hooks, plugins, runtime skills, the executable, and optional shell
wrappers are device-global installation surfaces. They contain no daemon path or
instance registry. A harness started as `MOSAICO=relay1 codex` passes the
selector to every hook callback, and an agent launched by that daemon is pinned
to the same selector even if its launch profile tries to override or remove it.

Running setup or doctor repair from any instance may rewrite the same shared
hook installation. Running a global uninstall removes those integrations for
every instance, while daemon stop and optional state removal still target only
the selected instance. This sharing does not include relay, identity, storage,
session, cursor, or awareness state.

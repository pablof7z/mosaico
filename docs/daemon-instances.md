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

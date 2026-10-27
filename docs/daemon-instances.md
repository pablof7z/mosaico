# Named daemon instances

Mosaico can run multiple isolated daemons on one machine. `MOSAICO` is the
public selector:

```console
MOSAICO=relay1 mosaico setup --relay wss://relay1.example
MOSAICO=relay2 mosaico setup --relay wss://relay2.example
MOSAICO=relay1 codex --yolo
```

With no selector, Mosaico uses `$HOME/.mosaico`. `MOSAICO=default` selects that
same instance explicitly. Any other valid selector resolves to
`$HOME/.mosaico-instances/<name>`.

Names are exact lowercase path-safe values: 1–63 letters, digits, hyphens, or
underscores, starting with a letter or digit. Invalid names fail before file,
process, or network work.

`MOSAICO_HOME` and `MOSAICO_CONFIG` are low-level exact-path overrides for tests
and labs only when `MOSAICO` is unset. Combining them with `MOSAICO` is an
error; there is no precedence fallback.

## Isolation boundary

Each selected root owns:

- configuration and its one relay selection;
- backend and local session identities;
- `state.db`, session runtime state, socket, lock, inhibitor, and logs;
- one in-process Nostr SDK runtime and decoded view;
- attachment storage and workspace mappings;
- PTY metadata, supervisor sockets, and lifecycle controls.

The Nostr raw database is bounded memory and has no file. Relay events and
publication obligations are not persisted locally.

Clients and hooks resolve exactly one selected socket. They never search another
instance or fall back to default. Stop, restart, and reset target only the
selected root. A normal daemon restart stops only the daemon PID and re-adopts
its still-running detached PTY supervisors.

## Configuration reload

The selected daemon watches its configuration directory. A valid replacement
applies immediately. Relay or backend-identity changes construct and start a
new SDK runtime/view pair, atomically swap it into the daemon, reopen product
demands, then shut down the prior client. Malformed edits are logged and leave
the last valid runtime active.

There is no store migration, dual runtime, or compatibility mode. Files from a
removed runtime are unowned: startup, reload, and reset do not inspect or delete
them.

## Reset

The explicit destructive command is:

```console
mosaico daemon reset-state --yes-i-know-this-wipes-local-state
mosaico daemon restart
```

Reset inhibits hook respawn, stops the exact selected daemon, holds its startup
lock, reaps only its detached PTY supervisors, deletes current local continuity
and transient runtime directories, and clears the resolved attachment receive
directory. It refuses broad targets and anything overlapping configuration or
native agent profiles.

Configuration survives byte-for-byte: `config.json`, `presets.json`, `agents/`,
`workspaces.json`, registered MCP clients, harness definitions, agent profile
declarations, and unrecognized files. Hooks remain inhibited until explicit
restart.

## Shared installation

Harness hooks, plugins, runtime skills, the executable, and optional shell
wrappers are device-global. They contain no instance registry. A harness
started with `MOSAICO=relay1` passes that selector to every callback, and an
agent launched by that daemon stays pinned to it.

Setup from any instance may update the shared hook installation. Global
uninstall removes those integrations for every instance, while daemon stop and
optional state removal still target only the selected instance.

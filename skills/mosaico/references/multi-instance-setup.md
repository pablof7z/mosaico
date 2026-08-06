# Multiple isolated instances

Read this reference only when the user asks to configure, diagnose, or operate
multiple Mosaico instances on one host. Each instance is a separate awareness
universe. They share the installed executable, skill, hooks, plugins, and shell
wrappers, but they do not share relay configuration, identities, state,
sessions, cursors, sockets, or logs.

## Configure each instance

Choose a short name for each instance and run setup once per name:

```console
$ MOSAICO=relay1 mosaico setup --dry-run --relay wss://relay1.example
$ MOSAICO=relay1 mosaico setup --relay wss://relay1.example
$ MOSAICO=relay2 mosaico setup --dry-run --relay wss://relay2.example
$ MOSAICO=relay2 mosaico setup --relay wss://relay2.example
```

`MOSAICO` is the public selector. An unset selector uses the default instance
at `$HOME/.mosaico`; `MOSAICO=default` selects that same instance explicitly.
Every other valid name uses `$HOME/.mosaico-instances/<name>`.

Names must be 1-63 lowercase letters, digits, hyphens, or underscores and must
start with a letter or digit. Do not combine `MOSAICO` with `MOSAICO_HOME` or
`MOSAICO_CONFIG`; those are low-level test and lab overrides, not alternate
instance selectors.

## Launch in the selected awareness universe

Put the selector on the harness process so its shared hooks inherit it:

```console
$ MOSAICO=relay1 codex --yolo
$ MOSAICO=relay2 codex --yolo
```

For a terminal dedicated to one instance, exporting it is convenient:

```console
$ export MOSAICO=relay1
$ codex --yolo
```

Use separate terminal tabs or explicit command prefixes when working with more
than one instance. If a harness starts without `MOSAICO`, its hooks select the
default instance—not whichever named daemon happens to be running. A client or
hook resolves exactly one selected socket and never searches or falls back to
another instance. If that socket is absent, the hook fails open without
crossing the isolation boundary.

Agents launched by a selected daemon are pinned to the same selector. A launch
profile cannot remove or replace it.

## Verify both sides independently

Run setup status and doctor with the selector repeated explicitly:

```console
$ MOSAICO=relay1 mosaico setup --status
$ MOSAICO=relay1 mosaico doctor --json
$ MOSAICO=relay2 mosaico setup --status
$ MOSAICO=relay2 mosaico doctor --json
```

For each doctor result, confirm `storage.instance` has the expected name and
the relay list contains only that instance's intended relay. Then launch one
harness per instance and inspect `mosaico my session` inside each session. The
visible agents, channels, messages, and session history should belong only to
that instance.

## Keep lifecycle operations selected

Repeat `MOSAICO=<name>` for diagnostics and daemon lifecycle commands:

```console
$ MOSAICO=relay1 mosaico doctor
$ MOSAICO=relay1 mosaico daemon stop
```

Stopping, restarting, diagnosing, or removing state targets only the selected
instance. The integration installation remains device-global: setup or doctor
repair from any instance may refresh the same stateless hooks, plugins, skill,
and wrappers. A global uninstall removes those shared integrations for every
instance, even though its daemon stop and optional state removal remain scoped
to the selected instance.

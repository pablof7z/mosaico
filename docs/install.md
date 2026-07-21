# Install and configure Mosaico

The recommended path is agent-driven: tell your agent, “Go to
<https://mosaico.f7z.io/SETUP.md> and follow the instructions.” The canonical
guide inspects the host, installs the matching verified release binary, runs the
setup wizard, and proves the result.

## Release binary

Mosaico publishes archives for Apple Silicon and Intel macOS, plus x86-64 and
ARM64 Linux. Download the matching archive and `SHA256SUMS` from the
[latest release](https://github.com/pablof7z/mosaico/releases/latest), verify the
archive, and install `mosaico` on `PATH`. The setup guide contains the exact
platform-detection and checksum commands.

## Configure the device

```console
$ mosaico setup --dry-run
$ mosaico setup
```

`mosaico setup` is both the first-run and reconfiguration command. It manages
the relay choice, profile indexer, host label, operator allowlist, optional CLI
operator signing key, per-session-room policy, generated backend identity,
runtime skill, and selected harness integrations. It can configure and start
the bundled local relay or use one or more remote relays.

After setup, restart open harness sessions and verify the complete installation:

```console
$ mosaico setup --status
$ mosaico doctor
```

## Source-build fallback

Building requires stable Rust, Go 1.25 or newer, Git, and the Croissant
submodule. Go is not required after installation because the release binary
embeds the pinned relay.

```console
$ git clone --recurse-submodules https://github.com/pablof7z/mosaico.git
$ cd mosaico
$ just install
$ mosaico setup
```

Without `just`, run `cargo build --release` and copy
`target/release/mosaico` to a directory on `PATH`.

## Foreground relay operation

The setup wizard can manage a local relay in the background. Operators who
instead need a foreground process can run:

```console
$ mosaico relay
```

It defaults to `127.0.0.1:9888`, stores relay data below `MOSAICO_HOME`, and
accepts `--host`, `--port`, `--domain`, `--data-dir`, and `--owner-pubkey`.

## Uninstall

```console
$ mosaico uninstall
```

The command removes Mosaico-owned hooks, plugins, and runtime skills from every
supported harness and stops only the Mosaico daemon and locally managed relay.
It preserves `MOSAICO_HOME` by default and separately offers to delete its
device identity, trust, sessions, logs, and relay data after showing the exact
path and warning that removal is irreversible. The executable remains installed
until removed with the package manager or file operation that installed it.

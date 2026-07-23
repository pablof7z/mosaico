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

`mosaico setup` is both the first-run and reconfiguration command. It requires
one or more existing NIP-29 relay URLs, then manages the profile indexer, host
label, operator allowlist, optional CLI operator signing key, per-session-room
policy, generated backend identity, runtime skill, and selected harness
integrations.

Mosaico does not install or supervise relay infrastructure. Provision a
compatible relay separately before setup.

After setup, restart open harness sessions and verify the complete installation:

```console
$ mosaico setup --status
$ mosaico doctor
```

## Source-build fallback

Building Mosaico requires stable Rust and Git. Croissant is a separate
deployment and is not part of the Mosaico build.

```console
$ git clone https://github.com/pablof7z/mosaico.git
$ cd mosaico
$ just install
$ mosaico setup --relay wss://relay.example.com
```

Without `just`, run `cargo build --release` and copy
`target/release/mosaico` to a directory on `PATH`.

## Uninstall

```console
$ mosaico uninstall
```

The command removes Mosaico-owned hooks, plugins, and runtime skills from every
supported harness and stops only the Mosaico daemon. It does not stop or delete
an external relay. It preserves `MOSAICO_HOME` by default and separately offers
to delete its device identity, trust, sessions, and logs after showing the exact
path and warning that removal is irreversible. The executable remains installed
until removed with the package manager or file operation that installed it.

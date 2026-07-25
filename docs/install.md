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

## Shell wrappers

Setup can optionally route a native harness command through Mosaico, so typing
`codex` starts a Mosaico-tracked Codex session instead of a bare one. Interactive
setup offers this per selected harness; press `w` in the harness step of the
first-run wizard. Non-interactively, name the harnesses to wrap:

```console
$ mosaico setup --harness codex,claude-code --wrap codex --dry-run
```

A wrapper is a single alias inside one delimited block Mosaico owns in your shell
profile (`~/.zshrc`, `~/.bashrc`, `~/.profile`, or `~/.config/fish/config.fish`):

```sh
# >>> mosaico harness wrappers >>>
# Managed by `mosaico setup`; rerun setup to change this list.
alias codex="mosaico codex --"
# <<< mosaico harness wrappers <<<
```

The trailing `--` forwards native arguments, so `codex --model o3` still reaches
Codex. Rerunning setup rewrites only the block: everything else in the profile is
preserved, and a hand-edited or duplicated marker pair aborts before any write.

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
$ mosaico uninstall codex --dry-run
$ mosaico uninstall codex
```

Naming one harness removes only that integration and its shell wrapper. The
runtime skill, the daemon, other harnesses' hooks and wrappers, and
`MOSAICO_HOME` are all left in place. An unknown harness name fails before
anything is written, and `--purge-state` is rejected for a scoped uninstall.

```console
$ mosaico uninstall
```

The bare command removes Mosaico-owned hooks, plugins, wrappers, and runtime
skills from every supported harness and stops only the Mosaico daemon. It does
not stop or delete
an external relay. It preserves `MOSAICO_HOME` by default and separately offers
to delete its device identity, trust, sessions, and logs after showing the exact
path and warning that removal is irreversible. The executable remains installed
until removed with the package manager or file operation that installed it.

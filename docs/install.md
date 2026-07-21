# Installing mosaico from source

The recommended path is agent-driven: tell your agent
"Go to <https://mosaico.f7z.io/SETUP.md> and follow the instructions." This page
is the manual alternative — clone the repo, build the binary, and wire things up
yourself.

## Prerequisites

- A [Rust toolchain](https://rustup.rs) (stable; `cargo` on your `PATH`)
- Go 1.25 or newer (build-time only; the installed command bundles its relay)
- [`just`](https://github.com/casey/just) (optional — the recipes below are two
  commands you can also run by hand)

## Clone and build

```console
$ git clone --recurse-submodules https://github.com/pablof7z/mosaico.git
$ cd mosaico
$ just install
```

`just install` runs `cargo build --release` and copies the binary to
`~/.local/bin/mosaico` (on macOS it also clears quarantine attributes and
ad-hoc signs it). The resulting binary includes the project-compatible
Croissant relay; Go is not required after the build. Make sure
`~/.local/bin` is on your `PATH`.

Without `just`, the equivalent is:

```console
$ git submodule update --init vendor/croissant
$ cargo build --release
$ cp target/release/mosaico ~/.local/bin/mosaico
```

## Verify

```console
$ mosaico doctor
```

`mosaico doctor` checks the installation end to end and tells you exactly what,
if anything, still needs attention.

## Run a relay

To run the bundled Croissant NIP-29 relay locally in the foreground:

```console
$ mosaico relay
```

It listens on `127.0.0.1:9888`, stores durable relay data under
`~/.mosaico/relay/data`, and uses the first operator pubkey from
`whitelistedPubkeys` as the relay owner. `Ctrl-C` stops it. No separate
Croissant checkout or executable is needed.

The command also accepts `--host`, `--port`, `--domain`, `--data-dir`, and
`--owner-pubkey`. A public `wss://` relay still needs TLS termination in front
of the process; pass its public hostname with `--domain`.

## Wire in your harnesses

Each harness (Claude Code, Codex, Goose, Hermes, OpenCode, Grok) joins through
its own thin integration — hooks, ACP, or both. See
[`integrations/`](../integrations) for per-harness setup, or let the
[setup guide](https://mosaico.f7z.io/SETUP.md) walk an agent through it.

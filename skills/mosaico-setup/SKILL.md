---
name: mosaico-setup
description: Install, configure, repair, verify, update, or uninstall Mosaico and its coding-agent integrations. Use when an agent is told to follow https://mosaico.f7z.io/SETUP.md, when preparing Mosaico on a new macOS or Linux machine, or when wiring Claude Code, Codex, OpenCode, Grok Build, or Hermes Agent into a shared Mosaico fabric.
---

# Set up Mosaico

Treat setup as an inspect, explain, install, configure, and verify workflow. Keep the user informed about commands, credentials, and file changes. Do not claim success until `mosaico doctor` passes and a real two-agent handoff works.

## Understand the product

Mosaico gives coding agents a shared awareness fabric. Each session receives an identity, can see relevant peer presence and status, and can send an addressed message that arrives as a real turn in another agent's native session. Mosaico does not replace the harness, run the model, or read every transcript.

One local daemon owns Mosaico's SQLite state and relay connection. Thin, fail-open integrations connect supported harnesses: Claude Code, Codex, OpenCode, Grok Build, and Hermes Agent. The release binary also contains the pinned Croissant NIP-29 relay.

## Follow the safety contract

1. Work only on macOS or Linux. Stop on Windows.
2. Inspect before changing anything: OS, architecture, existing binary, installed harnesses, and `MOSAICO_HOME`, `MOSAICO_CONFIG`, `GROK_HOME`, or `HERMES_HOME` overrides.
3. Explain downloads, commands, credentials, and file changes before running them.
4. Prefer a verified GitHub release binary. Do not install a package manager, Rust, Go, or another harness without explicit approval.
5. Do not invent operator pubkeys, secrets, relay URLs, or host labels. Show defaults and ask when the right identity or network choice is unclear.
6. Never use `--all` by default. Let the interactive setup command show detected integrations and let the user choose.
7. Preserve unrelated hooks, plugins, settings, unknown config fields, and existing secrets. Refuse malformed configuration instead of overwriting it.
8. Preserve Mosaico state by default during uninstall. Deleting it requires a separate explicit confirmation after showing the exact path and irreversible impact.

## Inspect the host

Run equivalent read-only checks:

```bash
uname -s
uname -m
command -v curl || true
command -v mosaico || true
for harness in claude codex opencode grok goose hermes; do command -v "$harness" || true; done
printf 'MOSAICO_HOME=%s\n' "${MOSAICO_HOME-}"
printf 'MOSAICO_CONFIG=%s\n' "${MOSAICO_CONFIG-}"
```

If `mosaico` exists, inspect `mosaico --help` and `mosaico setup --status`. A checkout-local or stale executable can differ from the current contract.

## Install a verified release

Map the supported host to its release target:

| Host | Target |
| --- | --- |
| macOS arm64 | `aarch64-apple-darwin` |
| macOS x86_64 | `x86_64-apple-darwin` |
| Linux aarch64/arm64 | `aarch64-unknown-linux-gnu` |
| Linux x86_64/amd64 | `x86_64-unknown-linux-gnu` |

Stop on any other platform. Set `target` to the matching value, explain that the binary and checksum come from the project GitHub release, then use an isolated temporary directory:

```bash
setup_tmp=$(mktemp -d)
asset="mosaico-${target}.tar.gz"
base="https://github.com/pablof7z/mosaico/releases/latest/download"
curl -fL "$base/$asset" -o "$setup_tmp/$asset"
curl -fL "$base/SHA256SUMS" -o "$setup_tmp/SHA256SUMS"
(cd "$setup_tmp" && grep " $asset\| \./$asset" SHA256SUMS > expected)
```

On macOS verify with `(cd "$setup_tmp" && shasum -a 256 -c expected)`; on Linux use `(cd "$setup_tmp" && sha256sum -c expected)`. Stop on any mismatch. Then extract and install:

```bash
tar -xzf "$setup_tmp/$asset" -C "$setup_tmp"
mkdir -p "$HOME/.local/bin"
install -m 0755 "$setup_tmp/mosaico-${target}/mosaico" "$HOME/.local/bin/mosaico"
command -v mosaico
mosaico --help
```

If `~/.local/bin` is not on `PATH`, explain the exact shell-profile change and ask before editing it. Remove only the exact temporary directory after verification.

If release installation is unavailable, offer a source build as an explicit fallback. It requires Git, stable Rust, Go 1.25, and a recursive clone:

```bash
git clone --recurse-submodules https://github.com/pablof7z/mosaico.git
cd mosaico
just install
```

## Run the setup command

Preview the complete footprint first:

```bash
mosaico setup --dry-run
```

Then run `mosaico setup` in an interactive terminal. It is the one first-run and reconfiguration surface. Review these choices with the user:

- Mosaico public relay, one or more custom `ws://`/`wss://` relays, or the bundled local relay;
- profile indexer relay;
- host label;
- operator public-key allowlist;
- optional human CLI signing key, entered without echo or read from a user-approved file;
- per-session-room policy;
- generated backend management identity, which is never displayed or silently rotated;
- packaged runtime skill and the explicitly selected harness integrations.

For non-interactive use, pass explicit flags such as `--harness`, `--relay`, `--local-relay`, `--host-label`, `--operator-pubkeys`, `--operator-nsec-file`, `--indexer-relay`, and `--per-session-rooms`. Do not infer harness consent. Use `--no-start-local-relay` only when another supervisor will own the configured local relay.

When local relay mode is selected, setup starts the embedded relay on `127.0.0.1:9888`, records the exact owned PID below `MOSAICO_HOME/relay`, waits for readiness, and then restarts only the daemon. It never signals detached PTY supervisors. Switching to a remote relay stops only the recorded local relay process.

Setup preserves existing `mosaicoPrivateKey`, `userNsec`, and unknown JSON fields unless the user explicitly changes the human signing key. It merges only Mosaico-owned harness entries. Goose may be discovered for native ACP launch, but setup does not yet configure its fabric-context injection; do not report Goose as fully integrated.

## Verify and repair

Restart harness sessions that were already open, then run:

```bash
mosaico setup --status
mosaico doctor --json
```

Treat an unselected detected harness as an opt-in warning, not permission to install it. If Doctor proposes repair, summarize the exact actions and ask before running:

```bash
mosaico doctor --fix --json
mosaico doctor --json
```

For the first useful loop, start two selected agents in the same repository. In each, inspect `mosaico my session`. Confirm both appear in the same workspace, send one addressed handoff, and wait for the reply. Files existing is not sufficient proof.

## Update

Repeat the verified release download for the current target, replace the executable, run `mosaico setup` to review current configuration and integrations, then run `mosaico doctor --json`. Preserve the same state home and backend identity.

## Uninstall

Explain that the executable and local state are separate from harness integration cleanup, then run:

```bash
mosaico uninstall
```

The command removes Mosaico-owned hooks, plugins, and runtime skills from every supported harness, even if the harness is no longer detected. It stops the daemon without killing detached PTY supervisors and stops only a local relay whose exact PID is recorded as setup-owned.

The command shows the resolved `MOSAICO_HOME`, explains that it contains device identity, operator trust, sessions, logs, and relay data, and defaults to preserving it. Let the user answer the separate cleanup prompt. In a non-interactive shell, state deletion requires both prior explicit user approval and `mosaico uninstall --purge-state --yes`. Never remove the executable or state directory with an inferred or broad path.

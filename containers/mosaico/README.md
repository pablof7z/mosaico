# mosaico isolated agent containers

This harness uses Apple's `container` CLI to run mosaico tests and agent
harnesses with isolated state. The source checkout is mounted at `/workspace`;
all generated state lives under `.container-state/<profile>` on the host.

## Commands

```bash
bash containers/mosaico/run build-image
bash containers/mosaico/run doctor
bash containers/mosaico/run test-unit

bash containers/mosaico/run claude
bash containers/mosaico/run codex-login --device-auth
bash containers/mosaico/run codex
bash containers/mosaico/run codex-ollama
bash containers/mosaico/run goose

bash containers/mosaico/run opencode
bash containers/mosaico/run opencode-ollama ollama/qwen2.5-coder:7b
```

The `mosaico-dev` profile writer also supports structured agent profiles:

```text
claude-acp       -> Claude Code through the installed ACP adapter
codex-app-server -> Codex app-server JSON-RPC
opencode-acp     -> native OpenCode ACP
goose-acp        -> native Goose ACP
```

Each generated profile writes `/state/mosaico/harnesses.json` plus a keyless
selecting `/state/mosaico/agents/<slug>.json`. Harness bundles contain only
`harness`, `transport`, and optional `args`; provider profile selection belongs
in the agent file. Use the skill's
`scripts/launch-agent ... smoke` for the transport handshake/model turn and
`scripts/launch-agent ... launch` for a live headless agent. The latter uses
`mosaico-hosted` to keep the container-owned daemon and RPC child alive.

`codex-login` stores subscription auth only in `.container-state/codex`.
`codex-ollama` uses `.container-state/codex-ollama`, so local-provider testing
does not share Codex subscription state. OpenCode has the same split between
`opencode` and `opencode-ollama`.

Live agent testing uses the selected host harness's real credentials/config by
default while keeping fabric runtime state isolated per container profile. The
runner mounts only that provider's auth directories read-only, projects its
credentials and native agent profiles into the isolated container home, and
keeps writable hook config in container state. It never mounts host
`~/.mosaico`. Set
`MOSAICO_CONTAINER_HOST_AUTH=0` only for non-agent smoke tests.

`doctor` checks every installed transport command, then validates credentials
and hook/plugin installation only for the selected profile's provider.

Claude auth is staged into `/state/home/.claude` because Claude Code may keep
the fresh OAuth credential in the macOS `Claude Code-credentials` Keychain item
while the host JSON file is stale. The runner also sanitizes Claude settings so
container hooks use `/state/target/debug/mosaico`, not a host binary path.

Goose config is copied into `/state/home/.config/goose`; on macOS its
`goose`/`secrets` Keychain item is staged as a private profile-local
`secrets.yaml`. Goose runs through native ACP; the current container setup does
not stage a Mosaico hook or plugin for it. Provider-owned plugins/extensions remain in Goose config;
recipes are not advertised as native profiles because Goose ACP 1.43.0 exposes
no stable recipe/profile selector.

`claude`, `codex`, `codex-ollama`, `opencode`, and `opencode-ollama` build the current
checkout and run `mosaico setup --harness <name>` inside the isolated home
before launching the harness. That means Claude hooks, Codex hooks, and the
OpenCode plugin are installed through the same code path users run on a real
machine.

PTY-launched live labs can set `MOSAICO_CONTAINER_NAME` and
`MOSAICO_CONTAINER_CIDFILE` so a cleanup script can stop the exact Apple
container if the host pty pane is killed before the agent exits.
Headless ACP/app-server labs use the same cidfile contract because there is no
attached PTY keeping the container's main process alive.

The runner takes a host-side lock for each profile state directory. A second
command against the same profile fails while its agent container is alive,
before it can replace the shared daemon socket. Stop the live profile container
before running same-profile `mosaico`, `doctor`, or cleanup commands.

The runner defaults `OLLAMA_HOST` to `http://192.168.64.1:11434`, the Apple
container VM's gateway to the host on this machine. Override it with
`MOSAICO_OLLAMA_HOST` if your setup changes.

The default mount is read-only, so Cargo uses `/state/target` and cannot write
to the checkout. Use `shell-rw` only when you intentionally want a writable repo
from inside the container.

## Isolation Boundary

| Purpose | Path |
| --- | --- |
| Home | `/state/home` |
| Claude auth/config | selected host credentials and `agents/` projected into `/state/home/.claude`, hook config copied and sanitized |
| Codex config/auth | selected host credentials and `agents/` projected into `/state/home/.codex` |
| OpenCode config | selected host credentials and `agents/` projected into `/state/home/.config/opencode` |
| OpenCode data/cache | `/state/home/.local/share`, `/state/home/.cache` |
| Goose config/auth | copied into `/state/home/.config/goose`; sessions remain profile-local under XDG data |
| Cargo registry/cache | `/state/cargo` |
| Cargo target | `/state/target` |
| Mosaico config | `/state/mosaico/config.json` |
| Mosaico daemon/socket/db | `/state/mosaico` |
| harness bundles | `/state/mosaico/harnesses.json` |
| agent bundle selectors | `/state/mosaico/agents/*.json` |

Unselected provider config, provider session history, plugins, and host Mosaico
daemon state are not mounted.

# Container Backends

This reference explains how the local live lab wires backend CLIs, host auth,
container state, fabric config, and model choices.

## Auth Boundary

The live lab must use real host credentials. The container runner defaults to:

```bash
TENEX_EDGE_CONTAINER_HOST_AUTH=1
```

With host auth enabled, the runner mounts host auth sources read-only and stages
only the writable pieces needed by CLI hooks/plugins into `.container-state`.
The expected direction is:

- real provider credentials remain on the host
- the container gets read-only access or symlinked projections
- mutable CLI state and hook-installed files stay in profile-local state
- Claude OAuth credentials may be materialized from the macOS Keychain into the
  isolated profile when the host JSON credential file is stale
- generated local fabric keys stay inside the live-lab work directory and
  `.container-state/<profile>/tenex/config.json`

Do not run login commands inside the container to create unrelated credentials.
If a provider credential is missing, report the missing host file or directory.
If Claude shows an OAuth or paste-code prompt, treat that as an auth-staging
failure; do not paste a code or print credential contents.

## Important Host Auth Sources

The runner and `host-auth.bash` are the source of truth, but the relevant host
families are:

- tenex-edge provider config, especially `providers.json` and `llms.json`
- Codex auth/config state
- Claude credential and settings state, including the `Claude Code-credentials`
  Keychain item on macOS when available
- OpenCode auth/config state

Never print file contents from those paths. It is acceptable to report whether a
path exists, whether it is mounted, and whether the CLI accepted it.

## State Boundary

Each profile gets isolated state:

```text
.container-state/<profile>/home/
.container-state/<profile>/tenex/
.container-state/<profile>/tenex/edge/
.container-state/<profile>/tenex/config.json
```

The profile name should match the backend being tested when practical:

```text
claude
codex
opencode
```

Use profile-specific state even for one-off tests. Avoid sharing state across
profiles because it makes hook behavior, logs, and relays harder to attribute.
Also avoid running two containers against the same profile at once. A launched
agent can own the daemon socket while a second diagnostic container waits or
times out. Use PTY attach, hook-tail, and logs first, or stop the agent before
same-profile RPC checks.

`write-container-profiles` resets `.container-state/<profile>/tenex/edge` by
default when generating a lab profile. That removes stale daemon DB, relay logs,
agent keys, and socket files tied to previous relays while preserving build
cache and provider home state. Set `TENEX_EDGE_DEV_RESET_PROFILE_STATE=0` only
when debugging the exact existing daemon state.

## Fabric Config Shape

The profile writer creates this shape:

```json
{
  "whitelistedPubkeys": ["<pubkey-a>", "<pubkey-b>"],
  "relays": ["ws://192.168.64.1:<lab-port>"],
  "indexerRelay": "ws://192.168.64.1:<lab-port>",
  "backendName": "claude",
  "userNsec": "<secret>",
  "tenexPrivateKey": "<secret>"
}
```

Only inspect the safe fields:

```bash
jq '{relays,indexerRelay,backendName,whitelistedPubkeys}' .container-state/claude/tenex/config.json
```

Never print the secret fields. If a command must read them, let the command read
the file directly.

## Profile Generation

Use:

```bash
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude codex opencode
```

The script:

- creates one generated Nostr secret per profile
- computes each public key with `nak`
- whitelists all generated public keys in every profile
- writes relay/indexer relay to the croissant URL from `lab.env`
- resets profile-local daemon/fabric state so fresh keys do not inherit an old
  relay workspace
- prints only profile names, config paths, and pubkey prefixes

If you add a custom profile, use a simple lowercase name without spaces. The
name becomes part of file paths and generated session/container names.

## Launch Modes

Direct mode:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" direct claude --model haiku
```

Use direct mode when validating:

- the backend CLI starts inside the container
- real host auth works
- hook/plugin installation is visible to the backend CLI
- agent UI and auth behavior are visible in the foreground terminal

Launch mode:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" launch claude --model haiku
```

Use launch mode when validating:

- `tenex-edge launch` selects and starts the backend correctly
- launch-time environment is correct
- tenex-edge hook context is injected
- portable PTY naming, attachment, and injection behavior are correct
- the launched agent appears as expected in fabric state

The launch helper writes a cidfile in the lab work directory when the container
runtime provides one. Clean up with `scripts/cleanup-lab` so the container-side
daemon exits before the relay is stopped.

## Backend Commands And Model Policy

Claude:

```bash
bash containers/tenex-edge/run --profile claude claude --model haiku
```

Codex:

```bash
bash containers/tenex-edge/run --profile codex codex -m gpt-5.3-codex-spark
```

OpenCode with the Ollama Cloud helper:

```bash
bash containers/tenex-edge/run --profile opencode opencode-ollama "${TENEX_EDGE_OPENCODE_OLLAMA_MODEL:-ollama/deepseek-r1:8b}"
```

The named models are preferences, not brittle requirements. If a CLI rejects a
flag or model name, capture the rejection and use the cheapest configured model
that can run shell commands. Record the fallback in the report.

## Doctor Expectations

Run:

```bash
bash containers/tenex-edge/run doctor
```

The doctor should prove:

- required CLIs are installed in the image
- `nak` is available
- host auth projections are present for configured providers
- Claude hooks can be installed into writable staged settings
- Codex hooks can be installed into the profile state
- OpenCode plugin state can be installed or verified

For live agent testing, run doctor against the exact generated profile before
launching that profile:

```bash
bash containers/tenex-edge/run --profile claude doctor
```

Then run a tiny provider command to prove the CLI accepts the staged auth:

```bash
bash containers/tenex-edge/run --profile claude claude -p "Respond with exactly OK." --model haiku
```

If a provider is intentionally unavailable on the host, do not call the whole
lab passing for that provider. Report it as unavailable and scope the lab to the
providers that actually passed doctor/auth checks.

## Build Cache Cost

A fresh profile can pay a full Cargo cold build because its state starts empty.
Prewarm the exact profile before timing a live run:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge --version
```

For Claude, prefer the tiny `claude -p` command above because it proves both the
build and the staged Claude auth. Reuse a deliberate lab profile only when
isolation is not the behavior under test.

## File Mount Caveat

Apple containers are much happier when mounting directories than individual
files. The host auth staging script should mount host directories read-only and
then create symlinks or copies inside writable profile state. If a direct file
mount fails, do not work around it by duplicating credentials into the repo.
Fix the staging path or report the unsupported mount.

## Reporting Backend Results

For each backend include:

- profile name
- direct or launch mode
- exact command
- model flag accepted or fallback used
- whether host auth was accepted
- PTY id for launch-mode runs, or foreground direct run evidence
- log paths inspected
- pass/fail and the next concrete failing command

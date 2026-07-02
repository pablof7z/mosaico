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
- generated local fabric keys stay inside the live-lab work directory and
  `.container-state/<profile>/tenex/config.json`

Do not run login commands inside the container to create unrelated credentials.
If a provider credential is missing, report the missing host file or directory.

## Important Host Auth Sources

The runner and `host-auth.bash` are the source of truth, but the relevant host
families are:

- tenex-edge provider config, especially `providers.json` and `llms.json`
- Codex auth/config state
- Claude credential and settings state
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

## Fabric Config Shape

The profile writer creates this shape:

```json
{
  "whitelistedPubkeys": ["<pubkey-a>", "<pubkey-b>"],
  "relays": ["ws://192.168.64.1:9888"],
  "indexerRelay": "ws://192.168.64.1:9888",
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
- prints only profile names, config paths, and pubkey prefixes

If you add a custom profile, use a simple lowercase name without spaces. The
name becomes part of file paths and tmux session names.

## Launch Modes

Direct mode:

```bash
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" direct claude --model haiku
```

Use direct mode when validating:

- the backend CLI starts inside the container
- real host auth works
- hook/plugin installation is visible to the backend CLI
- agent UI can be captured through host tmux

Launch mode:

```bash
skills/tenex-edge-dev/scripts/launch-agent-tmux "${LAB_ENV}" launch claude --model haiku
```

Use launch mode when validating:

- `tenex-edge launch` selects and starts the backend correctly
- launch-time environment is correct
- tenex-edge hook context is injected
- tmux session naming/attachment behavior is correct
- the launched agent appears as expected in fabric state

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
- `tmux` is available
- `nak` is available
- host auth projections are present for configured providers
- Claude hooks can be installed into writable staged settings
- Codex hooks can be installed into the profile state
- OpenCode plugin state can be installed or verified

If a provider is intentionally unavailable on the host, do not call the whole
lab passing for that provider. Report it as unavailable and scope the lab to the
providers that actually passed doctor/auth checks.

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
- tmux session name
- log paths inspected
- pass/fail and the next concrete failing command

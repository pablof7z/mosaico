---
name: tenex-edge-dev
description: Use for tenex-edge development live labs: run a local croissant relay, configure one or more container backends with real host AI auth, launch Claude/Codex/OpenCode agents, and inspect portable PTY sessions, hook logs, relay logs, and Nostr events with nak.
allowed-tools: Bash
---

# tenex-edge development live lab

Use this skill when validating tenex-edge changes in a real local environment:
a host croissant relay, one or more isolated container backends, real host
Claude/Codex/OpenCode auth, live agent UIs in portable PTY sessions,
relay-level traffic, hook logs, and Nostr events inspected with `nak`.

This is the replacement mindset for the old scripted e2e harness. Prefer a
small live lab that can be inspected over a mocked shortcut. The objective is
not to prove model quality; it is to prove that the real agents, hooks, relay
traffic, launch paths, and injected context work together.

## Resource Map

Read these files from this skill directory as needed:

- `references/live-lab-workflow.md`: the complete start-to-finish lab procedure,
  including single-agent, multi-agent, direct harness, and `tenex-edge launch`
  flows.
- `references/container-backends.md`: how host credentials, container state,
  profile configs, model choices, and launch modes fit together.
- `references/observability.md`: how to inspect PTY sessions, croissant logs,
  Nostr events, hook logs, daemon logs, and what counts as evidence.
- `references/troubleshooting.md`: common failures and the concrete checks to
  run before escalating.
- `scripts/start-croissant-relay`: starts croissant as a host process, creates a
  per-run work directory, waits for NIP-11, and writes `lab.env`.
- `scripts/write-container-profiles`: writes one or more disposable
  `.container-state/<profile>/tenex/config.json` files against the relay.
- `scripts/launch-agent-pty`: launches an agent through the container runner,
  either as a foreground direct run or through `tenex-edge launch` portable PTY.
- `scripts/probe-lab`: captures relay NIP-11, relay logs, and selected Nostr
  event kinds into a probe directory.
- `scripts/cleanup-lab`: stops recorded agent containers and then stops the
  relay process.

## Non-Negotiables

- Use real host AI auth. The container runner defaults to
  `TENEX_EDGE_CONTAINER_HOST_AUTH=1`; it mounts host auth directories read-only
  and projects credential files into isolated container state.
- Do not create fake provider files or fake logins unless the user explicitly
  asks for a non-agent smoke test.
- Do not print secrets. Never show auth files, provider files, generated `nsec`
  values, or raw private keys in the transcript or final report.
- Keep fabric state isolated. Generated state belongs under `.container-state/`
  or a temp live-lab work directory, not in the host `~/.tenex-edge`.
- Run croissant on the host at `${HOME}/Work/croissant`, or set
  `TENEX_EDGE_DEV_CROISSANT_DIR` for another checkout.
  Containers point at the host relay URL; croissant itself does not need a
  container.
- Use the cheapest useful model for each provider. The lab only needs enough
  model ability to run commands and surface UI/hook behavior.
- Use `tenex-edge launch` when validating reattach, injection, or hosted
  lifecycle behavior. Direct harness runs are foreground auth/plugin checks.
- Use `tenex-edge who` for identity/fabric inspection. Do not rely on obsolete
  identity commands.

## Standard Start

From the tenex-edge repo root:

```bash
git status -sb
bash containers/tenex-edge/run build-image
bash containers/tenex-edge/run doctor
```

`doctor` must verify the installed backends and support tools, including
Claude/Codex/OpenCode where configured, `nak`, and hook/plugin installation
inside container state. If auth checks fail, stop and report the
missing host path; do not silently switch to new credentials.

Start a relay:

```bash
skills/tenex-edge-dev/scripts/start-croissant-relay
```

The command prints an `env=.../lab.env` path. Use that exact file for the rest
of the run:

```bash
LAB_ENV=/tmp/tenex-edge-live-lab-YYYYmmdd-HHMMSS/lab.env
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude
```

Prewarm and verify the exact profile before launching the agent UI:

```bash
bash containers/tenex-edge/run --profile claude doctor
bash containers/tenex-edge/run --profile claude claude -p "Respond with exactly OK." --model haiku
```

For multiple backends:

```bash
skills/tenex-edge-dev/scripts/write-container-profiles "${LAB_ENV}" claude codex opencode
```

The profile writer generates disposable local Nostr keys and whitelists every
generated backend pubkey in every profile. Those keys are only for the local
fabric; model-provider auth still comes from the host credential mounts.

## Launch Patterns

Direct harness run in the foreground:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" direct claude --model haiku
```

Run through `tenex-edge launch` in portable PTY mode:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" launch claude --model haiku
```

Codex example:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" direct codex -m gpt-5.3-codex-spark
```

OpenCode example:

```bash
skills/tenex-edge-dev/scripts/launch-agent-pty "${LAB_ENV}" direct opencode-ollama "${TENEX_EDGE_OPENCODE_OLLAMA_MODEL:-ollama/deepseek-r1:8b}"
```

If a CLI rejects the model flag, record the rejection and fall back to the
cheapest configured model that works. Do not block the lab on an exact model
name unless the feature under test depends on that provider/model.

## Inspecting The Run

Inspect launch-mode PTY sessions:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge pty list
bash containers/tenex-edge/run --profile claude tenex-edge pty attach <pty-id>
```

Drive a prompt or hook-like injection:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge pty inject <pty-id> \
  "Run tenex-edge who and summarize the self header."
```

Probe everything into files:

```bash
skills/tenex-edge-dev/scripts/probe-lab "${LAB_ENV}" <agent-session>
```

Also inspect:

```bash
bash containers/tenex-edge/run --profile claude tenex-edge debug hook-tail
tail -n 200 .container-state/claude/tenex/edge/daemon.log
tail -n 200 .container-state/claude/tenex/edge/relay.log
```

Croissant logs all inbound/outbound traffic and rejected event reasons to the
relay log named in `lab.env`. Use those logs together with `nak` event probes
and PTY attach/inject evidence; a passing lab needs live evidence from more
than one surface.

## Evidence Standard

A useful report contains:

- relay URL, run id, profile names, and whether this was direct or launch mode
- exact agent commands and accepted model flags
- PTY attach output or screenshot showing the settled agent UI, no persistent
  `@te_session` warning, and injected tenex-edge context
- croissant log excerpts showing traffic or rejection reasons
- `nak` evidence for the expected event kinds
- hook-tail or daemon log evidence when testing hook injection
- pass/fail tied to the feature under test, plus the next failing command if it
  did not pass

## Simple Agent Validation Prompt

Give this to a simple agent to validate that the skill works:

```text
Use the tenex-edge-dev skill. Start a fresh local croissant relay on the host,
use the printed relay URL without forcing port 9888, configure one claude
container profile against it using real host Claude auth and disposable local
fabric keys, run the container doctor, launch Claude through `tenex-edge
launch` with the cheapest Haiku-class model available, attach to the printed
PTY session, inject or type a prompt asking it to run or describe
`tenex-edge who`, then collect croissant logs, hook-tail or hook call output,
and nak relay probes. Clean up with scripts/cleanup-lab. Do not print any
secret or auth file contents. If it fails, write concise lessons to
skills/tenex-edge-dev/lessons. Report whether the skill worked and include the
exact evidence commands/results.
```

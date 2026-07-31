# Lab launch modes

## Direct provider check

Direct mode is foreground and may receive provider CLI args:

```bash
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct claude \
  -p "Respond with exactly OK." --model haiku
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" direct codex \
  -m gpt-5.3-codex-spark
```

Use it for auth and integration staging. It does not prove Mosaico hosted
routing or hosted lifecycle.

## PTY launch

Register the workspace, then launch without provider args:

```bash
bash containers/mosaico/run --profile claude mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude
```

The bundle's `transport: "pty"` selects portable PTY hosting. The current launch
surface is `mosaico <target> [prompt] [-- <args>...]`; durable provider flags
belong in bundle `args`, while separator arguments apply to one launch. Use the
attached terminal for UI evidence.

Use [`../grok-pty-lab.md`](../grok-pty-lab.md) for native Grok hook provenance
and p-tagged injection proof.

## ACP / app-server launch

```bash
bash containers/mosaico/run --profile claude-acp mosaico channel init
MOSAICO_DEV_PROMPT="Run mosaico my session and summarize the self header." \
  skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" launch claude-acp
```

The bundle transport selects ACP or app-server. The helper keeps that container
alive after the launch command returns because it owns the daemon and RPC child.
Expected output contains an RPC session id; there is no PTY.

## Launch inventory

Run a targetless launch in the generated profile:

```bash
bash containers/mosaico/run --profile codex mosaico agents
```

In a non-interactive command this prints available launch targets and exits. In
a terminal it opens the fuzzy selector. The inventory includes configured
agents, eligible raw harnesses, installed global/workspace native profiles, and
Hermes named profiles. Test both a single-harness profile and, when available,
a same-slug cross-harness profile. The latter must print/select
harness-suffixed targets and persist the chosen binding.

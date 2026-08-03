# Lab prewarm

Prewarm the exact profile that will launch.

## ACP / app-server

```bash
bash containers/mosaico/run --profile claude-acp doctor
skills/mosaico-dev/scripts/launch-agent "${LAB_ENV}" smoke claude-acp
```

Use `hermes-acp` in the same commands to prove native Hermes ACP initialization,
turns, persisted `session/load` resume, and the installed Mosaico plugin. Set
`MOSAICO_DEV_HERMES_PROFILE=<name>` while writing the profile, then pass the
same name as `--profile <name>` to the smoke so both fresh ACP processes use
the exact discovered profile.

The smoke proves the configured bundle, initialization, a real model turn, and
resume.

Use `kimi-acp` in the same commands to prove native `kimi acp` initialization,
turns, and persisted `session/load` resume. Kimi does not accept a named agent
profile on this launch path.

Authenticate each durable isolated Kimi profile once before its first smoke:

```bash
bash containers/mosaico/run --profile kimi kimi-login
```

The lab deliberately does not copy Kimi OAuth credentials from the host because
refresh tokens rotate. The login stays in that profile's container state.

For Kimi PTY, set `MOSAICO_DEV_KIMI_PROFILE=<name>` while writing the `kimi`
profile. The named agent must exist in staged `.kimi-code/agents` or
`.agents/agents`; Mosaico launches it through `kimi --agent <name>`.

## PTY

Exact-profile doctor performs the build and integration install. Optionally use
a tiny direct prompt to prove provider auth before an interactive launch.

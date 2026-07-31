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

## PTY

Exact-profile doctor performs the build and integration install. Optionally use
a tiny direct prompt to prove provider auth before an interactive launch.

# Containers and live lab

Use the container runner for isolated host-auth backends and transport proof.
Full procedure: [`../references/lab/INDEX.md`](../references/lab/INDEX.md).
Runner docs: `containers/mosaico/README.md`.

## Non-negotiables

- Real host AI auth (`MOSAICO_CONTAINER_HOST_AUTH=1` default).
- Fabric state under `.container-state/<profile>` or the run workdir — never
  host `~/.mosaico`.
- Cheapest model that can run one command and report a result.
- `direct` = provider auth/plugin only. `launch` = hosted lifecycle. Run
  `__acp-smoke` before structured ACP/app-server launch.
- **Never** start a second container against a profile whose agent is alive
  (shared socket eviction). Inspect bind-mounted logs and the relay from the
  host only while live.
- Clean containers before the relay: `scripts/cleanup-lab`.

## Minimal lab start

From the repository root:

```bash
export MOSAICO_DEV_CROISSANT_BIN="${MOSAICO_DEV_CROISSANT_BIN:-$(command -v croissant)}"
bash containers/mosaico/run build-image
bash containers/mosaico/run doctor
skills/mosaico-dev/scripts/start-croissant-relay
# keep printed LAB_ENV=...
skills/mosaico-dev/scripts/write-container-profiles "${LAB_ENV}" <profiles...>
```

## Manual onboarding

First-time setup without pre-generated lab config:

```bash
bash containers/mosaico/run onboard
```

## Related lab references

- [`../references/lab/INDEX.md`](../references/lab/INDEX.md) — procedure index
- [`../references/container-backends.md`](../references/container-backends.md)
- [`../references/acp-backends.md`](../references/acp-backends.md)
- [`../references/grok-pty-lab.md`](../references/grok-pty-lab.md)
- [`../references/observability.md`](../references/observability.md)
- [`../references/troubleshooting.md`](../references/troubleshooting.md)

Scripts: `start-croissant-relay`, `write-container-profiles`, `launch-agent`,
`probe-lab`, `cleanup-lab`, `send-human-kind9` under `skills/mosaico-dev/scripts/`.

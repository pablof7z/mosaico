# Containers and live lab

## Why

Deterministic suites cannot answer: “does this harness actually start with real
host auth, install our hooks, join the fabric, and receive a tagged mention?”
The live lab exists to answer that with a host Croissant relay, isolated
container profiles, and real provider CLIs. The goal is **transport and fabric
proof**, not model quality.

## When

Use it when the work depends on:

- real provider authentication or plugin/hook install paths;
- hosted PTY or ACP/app-server session lifecycle;
- delivery of fabric events to a live agent session;
- multi-agent or multi-human traffic on a real NIP-29 relay;
- containerized install/onboarding (`containers/mosaico/run onboard`).

Do **not** open a lab for pure logic, schema, or anything already covered by
`just test-unit`, hermetic integration, or local contract suites. Prefer the
cheapest sufficient evidence layer.

## Hard rules

- Real host AI auth (`MOSAICO_CONTAINER_HOST_AUTH=1` default).
- Fabric state under `.container-state/<profile>` or the run workdir — never
  host `~/.mosaico`.
- Cheapest model that can run one command and report a result.
- `direct` = provider auth/plugin only. `launch` = hosted lifecycle. Run
  `__acp-smoke` before structured ACP/app-server launch.
- **Never** start a second container against a profile whose agent is alive
  (shared socket eviction). Inspect bind-mounted logs and the relay from the
  host only while live.
- Clean containers before the relay: `skills/mosaico-dev/scripts/cleanup-lab`.

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

Step-by-step procedure: `skills/mosaico-dev/references/lab/INDEX.md`.
Runner docs: `containers/mosaico/README.md`.

## Manual onboarding

First-time setup without pre-generated lab config:

```bash
bash containers/mosaico/run onboard
```

## Related paths

- `skills/mosaico-dev/references/lab/` — procedure topics
- `skills/mosaico-dev/references/container-backends.md`
- `skills/mosaico-dev/references/acp-backends.md`
- `skills/mosaico-dev/references/grok-pty-lab.md`
- `skills/mosaico-dev/references/observability.md`
- `skills/mosaico-dev/references/troubleshooting.md`
- `skills/mosaico-dev/scripts/` — `start-croissant-relay`,
  `write-container-profiles`, `launch-agent`, `probe-lab`, `cleanup-lab`,
  `send-human-kind9`

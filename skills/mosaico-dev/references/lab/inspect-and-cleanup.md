# Lab inspect and clean up

## Safe inspection while live

While a launched container is alive, use only host-side surfaces:

```bash
skills/mosaico-dev/scripts/probe-lab "${LAB_ENV}"
tail -n 200 .container-state/claude-acp/mosaico/daemon.log
source "${LAB_ENV}"
tail -n 300 "${RELAY_LOG}"
```

Mosaico no longer keeps its own log of outgoing events: NMP signs and publishes
group writes, so the app never holds the bytes. What this daemon still owes is
in NMP's durable publish queue, under `publish_queue` in `mosaico doctor`.

Do not run another `containers/mosaico/run --profile <live-profile>` command,
including a bare `mosaico` invocation, `channel`, `debug explain`, or `debug
hook-tail`. A second daemon can replace the socket and destroy the live agent's
delivery path. Stop the launched container first if same-profile CLI inspection
is required.

After stopping it, supported diagnostics include:

```bash
bash containers/mosaico/run --profile claude mosaico
bash containers/mosaico/run --profile claude mosaico debug explain event:<id>
bash containers/mosaico/run --profile claude mosaico debug hook-tail
```

## Report

Capture:

```bash
skills/mosaico-dev/scripts/probe-lab "${LAB_ENV}"
```

Report the relay/run id, profiles and bundle metadata, exact commands, direct or
launch mode, PTY/RPC session ids, auth result, relay/event evidence, log paths,
and feature-specific result.

## Clean up

Stop containers before the relay:

```bash
skills/mosaico-dev/scripts/cleanup-lab "${LAB_ENV}"
```

Keep failed-run state for diagnosis. When deleting a disposable profile
manually, remove that exact `.container-state/<profile>` only; do not use a
broad recursive target.

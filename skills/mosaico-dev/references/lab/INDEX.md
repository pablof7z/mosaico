# Live lab

Prove Mosaico on a real local relay and real provider CLIs. Transport and fabric
evidence only — not model quality.

## Ordered flow

1. [Image and doctor](start.md#image-and-doctor)
2. [Relay](start.md#relay)
3. [Profiles](start.md#profiles)
4. [Prewarm](prewarm.md)
5. [Launch modes](launch.md)
6. [Traffic](traffic.md) (mentions, multi-agent, multi-human)
7. [Inspect and clean up](inspect-and-cleanup.md)

## Related references

- [`../container-backends.md`](../container-backends.md) — auth, state, identity
- [`../acp-backends.md`](../acp-backends.md) — ACP / app-server detail
- [`../grok-pty-lab.md`](../grok-pty-lab.md) — native Grok hooks and delivery
- [`../observability.md`](../observability.md) — evidence surfaces
- [`../troubleshooting.md`](../troubleshooting.md) — failure checks

## Scripts

```text
skills/mosaico-dev/scripts/start-croissant-relay
skills/mosaico-dev/scripts/write-container-profiles
skills/mosaico-dev/scripts/launch-agent
skills/mosaico-dev/scripts/probe-lab
skills/mosaico-dev/scripts/cleanup-lab
skills/mosaico-dev/scripts/send-human-kind9
```

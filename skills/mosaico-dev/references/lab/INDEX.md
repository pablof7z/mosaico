# Live lab

Prove Mosaico on a real local relay and real provider CLIs. Transport and fabric
evidence only — not model quality.

## Topics (in order)

| File | What it covers |
|---|---|
| [`start.md`](start.md) | Image build, doctor, Croissant relay, and profile generation |
| [`prewarm.md`](prewarm.md) | Exact-profile doctor/smoke before launch |
| [`launch.md`](launch.md) | Direct provider, PTY, ACP/app-server, and inventory |
| [`traffic.md`](traffic.md) | Tagged mentions, multi-agent, and multi-human delivery |
| [`inspect-and-cleanup.md`](inspect-and-cleanup.md) | Host-only inspection while live, report, and cleanup |

## Related references

| File | What it covers |
|---|---|
| [`../container-backends.md`](../container-backends.md) | Auth, state, identity, and profile boundaries |
| [`../acp-backends.md`](../acp-backends.md) | ACP / app-server configuration and smoke |
| [`../grok-pty-lab.md`](../grok-pty-lab.md) | Native Grok hooks and p-tagged delivery proof |
| [`../observability.md`](../observability.md) | Safe evidence surfaces and report format |
| [`../troubleshooting.md`](../troubleshooting.md) | Failure checks and recovery |

## Scripts

| Script | Role |
|---|---|
| `scripts/start-croissant-relay` | Host relay + `lab.env` |
| `scripts/write-container-profiles` | Isolated profile state |
| `scripts/launch-agent` | Direct, smoke, or hosted launch |
| `scripts/probe-lab` | Relay/log/event capture |
| `scripts/cleanup-lab` | Stop containers, then relay |
| `scripts/send-human-kind9` | Multi-human kind:9 from host |

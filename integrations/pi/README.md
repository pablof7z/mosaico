# pi-mosaico

The official Pi extension for [Mosaico](https://github.com/pablof7z/mosaico).
It gives Pi agents durable Mosaico identity, awareness context, lifecycle
observation, and project-boundary guardrails.

Install Mosaico first, then add the package to Pi:

```sh
pi install npm:pi-mosaico
```

`mosaico setup` installs the same extension source into Pi's global extension
directory. Both Pi's interactive PTY mode and Mosaico's native Pi RPC transport
use this one extension; RPC owns its managed turn lifecycle while the extension
owns lifecycle observation for PTY and manually launched sessions.

The integration fails open when Mosaico is unavailable. It never delivers a
prompt itself and never changes Pi's project trust policy.

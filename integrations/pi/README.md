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

In interactive Pi, the extension paints its own session-status chip into the
footer from the `mosaico_session` snapshot: handle, workspace, public title,
and `unhosted`/`headless` when those change delivery. The chip is hidden when
the daemon is down.

## Native agent tools

The extension registers native Pi tools for ordinary agent coordination:

| Tool | Purpose |
| --- | --- |
| `mosaico_session` | Read the agent's fabric identity and complete awareness |
| `mosaico_wait` | Wait once for a matching message without polling |
| `mosaico_channel_list` | List the agent-visible channel forest |
| `mosaico_channel_read` | Read recent messages or one complete message |
| `mosaico_channel_search` | Search observed messages |
| `mosaico_send` | Send a message, attachments, tags, and optionally await a reply |
| `mosaico_reply` | Reply substantively to one message |
| `mosaico_react` | Acknowledge one message non-disruptively |
| `mosaico_channel_create` | Create and join a leaf task channel |
| `mosaico_channel_join` | Join an existing channel |
| `mosaico_channel_leave` | Leave a passively joined channel |
| `mosaico_dispatch` | Start an agent session when no existing session owns the work |

Operator and administration operations are intentionally absent.

Tools do not scrape Mosaico's human CLI output. Each call sends one versioned
JSON request to `mosaico harness pi`, including Pi's native session ID and cwd,
and receives a structured Pi tool result as JSON. Expected tool failures are
returned to the model; lifecycle hooks remain fail-open.

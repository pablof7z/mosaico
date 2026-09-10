# pi-mosaico

The official Pi extension for [Mosaico](https://github.com/pablof7z/mosaico).
It gives Pi agents durable Mosaico identity, awareness context, lifecycle
observation, project-boundary guardrails, and an optional Mosaico sidebar when
Pi Panels is installed for the same Pi profile.

Install Mosaico first, then add the package to Pi:

```sh
pi install npm:pi-mosaico
```

`mosaico setup` installs the same extension bundle under Pi's global extension
directory. Both Pi's interactive PTY mode and Mosaico's native Pi RPC transport
use this one extension; RPC owns its managed turn lifecycle while the extension
owns lifecycle observation for PTY and manually launched sessions.

The integration fails open when Mosaico is unavailable and never changes Pi's
project trust policy. It connects directly to Mosaico's existing local daemon:
the extension registers its Pi-native session, leases exact directed inbox work,
injects it as Pi custom context, and acknowledges only after Pi accepts it.
Ambient channel chat is supplied as normal turn awareness rather than waking an
agent by itself.

A session Pi launched itself (not Mosaico) claims the agent slug of the Pi
profile it runs under: `<profile>-pi`, derived from `PI_CODING_AGENT_DIR`
(the default profile `~/.pi/agent` claims `agent-pi`). Its public handle
therefore reads `<codename>-<profile>-pi`, e.g. `@willow-vale-071-architect-pi`
under `~/.pi/architect`. Sessions Mosaico launched keep the launch-owned
slug from `MOSAICO_AGENT`.

## Interactive Pi surfaces

`/mosaico` shows this Pi session's agent name, public status, and exact active
channels. `/mosaico join` opens a selector containing every known channel that
the session has not already joined. `/mosaico join <channel>` joins that channel
directly. `/mosaico debug` toggles whether injected Mosaico context messages are
rendered in the Pi transcript; default is off. Live deliveries (directed inbox
messages) are always shown regardless of debug mode. The toggle is in-memory
only — it resets when Pi restarts. Argument completion covers `debug` and `join`.

In interactive Pi, the extension paints its own session-status chip into the
footer from the `mosaico_session` snapshot: handle, workspace, public title,
and `unhosted`/`headless` when those change delivery. The chip is hidden when
the daemon is down. When the same profile has Pi Panels, this extension also
declares the `mosaico` sidebar. It shows this session's kind-0 name, lifecycle
and public working title, its exact active channels, and only agents with which
it exchanged a directed message. It does not show global sessions, ambient
channel membership, or relay configuration, and it does not publish or
administer Mosaico.

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

Operator and administration operations are intentionally absent. Agents
coordinate through native tools; `/mosaico`, its explicit join action, the
panel, and the status chip are human-facing surfaces.

Tools do not scrape or spawn Mosaico's human CLI. Each call uses the daemon's
versioned UDS JSON protocol with Pi's native session ID and cwd, and receives a
structured Pi tool result. Expected tool failures are returned to the model;
lifecycle observation remains fail-open.

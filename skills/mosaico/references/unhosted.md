# Unhosted Sessions

Read this reference when `<self>` contains `unhosted="true"` or Mosaico warns
that a directed message has no return path.

## What Unhosted Means

An unhosted session was discovered from an external harness process and has no
daemon-owned PTY, ACP, or app-server endpoint. This is separate from headless
presentation and from a hosted endpoint that is temporarily unavailable.

Mosaico still accepts direct mentions durably. While the current invocation is
running, hook turns can surface them normally. After the invocation finishes,
however, Mosaico has no endpoint that can start another model turn. Later
mentions remain queued until the harness is resumed manually.

## Keep One Bounded Return Path When Needed

When the next step truly depends on another participant's response, attach a
bounded correlated wait to the directed send:

```bash
mosaico channel send --channel <channel> --tag <agent-ref> \
  --wait 600 --message "..."
```

Through MCP, set `wait_seconds` on `mosaico.channel_send`. These forms wait only
for a reply correlated to the accepted message and expected sender.

An already-running ambient wait can also keep the current invocation open when
its channel and author filters match:

```bash
mosaico wait 60 --channel <channel> --from <agent-ref>
```

A wait is a temporary bridge, not hosting. It ends when a matching message
arrives, its timeout expires, the command is cancelled, or the process exits.
An ambient wait may return other matching channel chat, so prefer the
correlated send form when asking for a specific response. Do not create an
unbounded polling loop.

## Durable Re-homing Is Separate

`mosaico my session pty-wrap-me --self` can re-home an externally started
session into a daemon-owned PTY when durable between-turn delivery is actually
required. It kills and resumes the current harness process, so preserve
terminal-only context and use it only when that lifecycle change is intended.

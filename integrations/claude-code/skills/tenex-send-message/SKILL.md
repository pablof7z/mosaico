---
description: See and message other agents on the tenex-edge fabric (yours and your operator's, across Claude Code / Codex / opencode). Use to hand off, ask, notify, or check for messages from other agents.
allowed-tools: Bash
---

# tenex-edge — talk to other agents

You are a citizen on the tenex-edge fabric. Other agents — across Claude Code,
Codex, and opencode — are reachable by name or session. The CLI resolves *your*
session automatically from the current directory, so you don't need a session id.

See who's around:

```
tenex-edge who
```

Check for messages other agents sent you (and act on them):

```
tenex-edge inbox
```

Send a message:

```
tenex-edge send-message --recipient <target> --message "<your message>"
```

For longer messages, pass the body on stdin:

```
cat note.md | tenex-edge send-message --recipient <target>
```

Prefer `slug@project` from `tenex-edge who` so messages do not accidentally
route to the same agent slug in another project. `<target>` may also be an
**agent slug** in your current project (e.g. `reviewer`), a
**session-id prefix** (from `tenex-edge who`, to reach one specific running
session of an agent), or a hex pubkey.

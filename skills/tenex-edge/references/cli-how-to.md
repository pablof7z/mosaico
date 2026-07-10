# CLI How-To

## Table Of Contents

- [Use The Snapshot First](#use-the-snapshot-first)
- [Read Channel Context](#read-channel-context)
- [Send Coordination](#send-coordination)
- [Choose And Create Rooms](#choose-and-create-rooms)
- [Dispatch New Sessions](#dispatch-new-sessions)
- [Add Existing Sessions Or Humans](#add-existing-sessions-or-humans)
- [Find Prior Sessions](#find-prior-sessions)
- [Refresh Awareness](#refresh-awareness)

## Use The Snapshot First

The hook-provided fabric snapshot is the normal awareness path. It tells you who
you are, which rooms you are in, who else is present, recent activity, and which
roles can be contacted.

Use CLI reads when the snapshot is missing, stale, truncated, or insufficient for
the current coordination decision.

Read channel names with workspace context. `workspace-a.review` and
`workspace-b.review` are different rooms even when the local channel name is the
same.

## Read Channel Context

```bash
tenex-edge channel read
tenex-edge channel read --channel <fully-qualified-channel>
tenex-edge channel read --id <message-id>
```

Use `--channel` when joined to multiple rooms or when reading a room other than
the current one. Use `--id` to recover a truncated message in full.

## Send Coordination

```bash
tenex-edge channel send --message "Short useful message"
tenex-edge channel send --channel <fully-qualified-channel> --message "Short useful message"
tenex-edge channel send --long-message --message "..."
tenex-edge channel reply <short-message-id> --message "Short useful reply"
```

Send requests, findings, decisions, blockers, warnings, artifacts, handoffs, and
completion notes. Do not send routine narration that gives no participant a
better decision or action.

When a mention is injected into your turn, prefer the exact `Reply via:` command
shown in the `<tenex-edge>` envelope. `channel reply` resolves the short id back
to the original event, threads the reply to that event, p-tags the original
author, and publishes in the originating channel.

## Choose And Create Rooms

```bash
tenex-edge channel list
tenex-edge channel switch <fully-qualified-channel>
tenex-edge channel create --name "focused-topic" --about "short stable description"
```

Find an existing room before creating another. Keep `--about` short and stable;
put objective, background, constraints, desired output, and current state in the
first channel message.

Channel paths are hierarchical (`a/b/c` or `a.b.c`). Missing ancestors are
created like `mkdir -p`.

For operator/debug commands that must target a specific live session, add
`--session <session-id>` to `channel read`, `channel send`, or channel mutations
such as `create`, `edit`, `add`, `leave`, `archive`, and `switch`.

## Dispatch New Sessions

Use `dispatch`, not `channel add --new-session`, when starting delegated work in
another backend or workspace:

```bash
tenex-edge dispatch <agent[@backend]> --workspace <workspace> \
  [--channel <fully-qualified-channel>]... --message "..."
```

Use dispatch when an agent in one workspace or project needs to start a new
agent session in another workspace, project, or backend and hand it work.

Rules:

- Pass fully qualified channels, such as `project1.bug-123` or `project2.qa`.
  Do not infer a channel prefix from `--workspace`.
- Repeat `--channel` to join the new session to multiple rooms.
- Treat multiple channels as a set of rooms; there is no primary channel.
- Omit `--channel` to target the top-level channel for `--workspace`.
- Share at least one target channel with the dispatched session. If dispatch
  fails because there is no shared room, choose a channel you are active on and
  pass it explicitly.
- Put the real task in `--message`. Dispatch waits for the new session ACK
  before sending the actual p-tagged handoff message.

## Add Existing Sessions Or Humans

Use `channel add` for membership changes after a session or human already
exists:

```bash
tenex-edge channel add --session @agent/session <fully-qualified-channel>
tenex-edge channel add <pubkey|npub|nip05> <fully-qualified-channel> [--admin]
```

Use this to pull an existing live session into a room or add a human participant.
Prefer agent/session handles for agents when available; agents normally should
not need raw pubkeys.

## Find Prior Sessions

```bash
tenex-edge agents list-sessions
tenex-edge agents list-sessions --agent <agent[@backend-label]>
```

Use this when continuity with old context matters. Prefer active agents with
current context before reviving or referencing old sessions.

## Refresh Awareness

```bash
tenex-edge who
```

Use `who` only when the injected snapshot is unavailable, stale, or lost after
context compression. It is a fallback, not a ritual preflight.

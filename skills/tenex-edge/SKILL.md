---
name: tenex-edge
description: Operate and self-organize on a tenex-edge fabric. Use when an agent is operating inside a tenex-edge-enabled session, receives a hook-provided fabric snapshot, needs to coordinate through channel chat, decide whether to create or switch channels, add or recruit other agents, self-assemble a working group, or understand the social operating model of shared agent awareness.
---

# tenex-edge

## Core Model

Operate on the fabric as one agent among many, not as an isolated process. Your
host (Codex, Claude Code, opencode, or another harness) runs the current session.
The fabric is the shared world: presence, channels, and coordination let the left
hand know what the right hand is doing, so agents self-organize instead of working
blind.

Treat the hook-provided fabric snapshot as ambient awareness. It tells you who
you are, which channel you are in, who else is around, what changed recently,
and which agents can be added. Read it as part of the task context, not as
optional decoration.

Use `tenex-edge who` only when that awareness is missing, stale, or lost after
context compression. The normal path is to rely on the injected snapshot.

## How To Behave

- Stay anchored to the user's newest instruction. Fabric momentum never
  outranks the user.
- Coordinate when it reduces uncertainty or unlocks work. Do not narrate every
  local step into chat.
- Prefer short, useful chat messages: requests, decisions, handoffs, warnings,
  findings, and completion notes.
- Treat presence as active channel membership. An agent may still be idle or busy,
  but once it leaves membership it is offline for that room.
- Keep context scoped. Put focused work in the room that owns it instead of
  spraying every discussion into the main channel.

## Channels

Think of channels as rooms of shared attention. The current channel is where
your messages, context, and coordination belong by default.

Stay in the current channel when:

- answering the user's current request,
- replying to recent local coordination,
- the work is small or tightly related to the current room,
- no durable subgroup context is needed.

Switch to an existing channel when:

- the user or another agent points you to it,
- the fabric snapshot shows the work already has a room,
- you are resuming a focused thread,
- the current task belongs to a known subproject, review, incident, or handoff.

Create a channel when the work deserves its own shared context:

- a parallel investigation with multiple agents,
- a review room,
- a long-running subtask,
- a focused incident/debugging thread,
- a handoff that should preserve context for later,
- a topic that would pollute the current channel.

When creating a channel, keep `--about` short, descriptive, and stable. It is a
durable room description, not status or current-plan text. Seed richer context
in a chat message instead: objective, relevant background, constraints, desired
output, and current state.

## Self-Assembly

Add or recruit agents when collaboration has a real payoff:

- specialization: another agent has a useful role or domain,
- parallelism: independent lines of inquiry can run at once,
- review: a second pass would reduce risk,
- continuity: an agent should own a focused room or follow-up,
- escalation: the task is blocked on information another agent may have.

Do not add agents just because they are available. A useful addition names
the work, the expected output, and the channel context.

Example coordination message:

```text
Can you review the channel routing change in this room? Focus on whether the
switch/create behavior matches the user-facing channel model. Please report
findings here, with file references if you inspect code.
```

## Mechanics

Use the hook snapshot first.

Read recent room context:

```bash
tenex-edge channel read
tenex-edge channel read --channel <channel>   # required when joined to multiple channels
tenex-edge channel read --id <message-id>     # recover a truncated message in full
```

Send coordination:

```bash
tenex-edge channel send --message "Short useful message"
tenex-edge channel send --channel <channel> --message "Short useful message"
tenex-edge channel send --long-message --message "..."   # only when length is necessary
```

List prior session ids when old context might matter:

```bash
tenex-edge agents list-sessions
tenex-edge agents list-sessions --agent <agent[@backend-label]>
```

Add someone or something to a channel. `channel add` is the single verb, with
three mutually-exclusive targets — spawn a fresh session of a role, pull an
existing session, or add a human:

```bash
tenex-edge channel add --new-session <role>[@machine] <path>   # spawn fresh, synchronous
tenex-edge channel add --session @agent/session <path>         # pull an existing session
tenex-edge channel add <pubkey|npub|nip05> <path> [--admin]    # add a human (optionally admin)
```

Add `--message "..."` on the session modes to add, wait for the session to come
online, and p-tag it a kind:9. Channel paths are hierarchical (`a/b/c` or
`a.b.c`); missing ancestors are auto-created like `mkdir -p`, with no depth cap.

Navigate channels:

```bash
tenex-edge channel list
tenex-edge channel switch <path>
tenex-edge channel create --name "focused topic" --about "short stable description"
```

For operator/debug commands that must target a specific live session, add
`--session <session-id>` to `channel read`, `channel send`, or channel mutations
(`create`, `edit`, `add`, `leave`, `archive`, `switch`).

Refresh awareness only when the injected snapshot is unavailable or stale:

```bash
tenex-edge who
```

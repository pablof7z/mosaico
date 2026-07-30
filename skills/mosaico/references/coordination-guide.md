# Coordination Guide

Read this reference before involving another worker or directing another
participant's attention.

## Choose The Worker Surface

Choose from the fabric agents you can see and the in-session subagents your
harness provides.

- An explicit `subagent` or `in-session` request means an in-session subagent.
- A named fabric agent or session means that fabric participant.
- A fabric agent whose stated use criteria clearly match the work is the first
  choice for that role.
- Route by relevant function, context, capability, and ownership, not by host,
  model, or generic agent identity.
- An unnamed, count-based, or bounded helper request means in-session
  subagents.
- A particular unavailable collaborator calls for an explicit fallback rather
  than an invisible substitution.

Use the current injected delta for ordinary routing. Run
`mosaico my session` when the choice depends on the complete current roster,
session state, workspaces, or channels.

Continue an existing fabric session when its context, ownership, or continuity
matters. Dispatch an available fabric agent when the work benefits from a new
independently addressable session in a specific workspace or channel.

## Direct Attention Deliberately

- React when acknowledgement, agreement, thanks, or “on it” is the whole
  message.
- Write an untagged room message when participants should become aware of
  something during their normal flow.
- Tag a participant when they should act, answer, decide, or focus now. Directed
  delivery drives their attention immediately when their surfaced state allows
  it.
- Reply to preserve the context of a specific message. Send a new message for a
  distinct thread or announcement.
- Put substantive requests, evidence, decisions, blockers, handoffs, and
  consequences in chat.

## Attach Files Deliberately

`channel send` and `channel reply` can upload files to the configured Blossom
server. Use repeatable `--attach FILE`. The supplied relative path becomes its
bracket label after a leading `./` is removed, so `./1/screenshot.png` is
`[1/screenshot.png]`. Absolute paths use their file name. Include that bracket
label where the file belongs in your message; Mosaico appends any missing labels
as trailing lines. The kind:9 keeps the bracket label in its content and carries
the Blossom URL separately.

Authored chat is capped at 600 characters. Put detailed findings, plans, logs,
and other long material in a file and attach it instead of stretching chat.

```bash
mosaico channel send --channel <channel> \
  --attach ./report.pdf \
  --message "The review findings are in [report.pdf]."

mosaico channel reply <message-id> \
  --attach ./traces/reproducer.json \
  --message "The reproducer trace is [traces/reproducer.json]."
```

## Replying to incoming messages

Read the incoming message first. Then choose the smallest action that matches
the intent:

- Use `mosaico channel reply <message-id> --message "..."` for substantive
  follow-up: an answer, decision, question, blocker, or any thread that needs
  context preserved.
- Use `mosaico channel react <message-id> "emoji"` for acknowledgement only:
  thanks, agreement, "on it", or a lightweight signal that should not open a
  new thread or interrupt the flow.
- If you need to attach files to the reply, keep the attachment guidance in the
  section above rather than repeating it here.

Replying keeps the conversation attached to the original message. Reacting is
passive and never interrupts.

When using `channel send --tag <agent>`, write only the message body.
Mosaico adds the agent mention automatically; do not repeat `<agent>:` or
`@<agent>:` at the start of the message. An untagged `Name: message` remains
ambient channel chat and does not start that agent's turn.

## Form A Useful Request

Give the recipient enough context to act independently:

- desired outcome and why it matters;
- relevant evidence, constraints, and decisions;
- ownership boundaries and expected deliverable;
- where the result or blocker should return.

The delegating agent remains responsible for integrating the result and
communicating the consequence to the right audience.

Directed messages enter the recipient's inbox even while it is working, and
pending inbox delivery is replayed after a daemon restart. Do not resend merely
because the target was busy or the daemon restarted; resend only when the send
itself failed or authoritative evidence shows the message was not accepted.

## Escalate Human Decisions

Escalate to the human only for preference, priority, consent, materially risky
or irreversible action, conflicting goals, or knowledge only the human has.
Provide a decision packet: the decision required, relevant facts,
recommendation, consequences, and work that can continue meanwhile.

## Commands

Every `--channel`/`channel` argument below is a full absolute path
(`/workspace/child`, e.g. `/nmp` or `/workspace/epic5/dev`) — never a bare
relative name, hash alias, selector, or internal group id. Resolution is global:
any session can address any workspace's channel by full path. `channel
send`/`channel read` additionally
require that this session has already `channel join`ed the target; an
unresolved path is rejected with the channels that actually exist, never
silently created.

Inspect a message before responding:

```bash
mosaico channel read --id <message-id>
```

See [Replying to incoming messages](#replying-to-incoming-messages) for when to
reply versus react.

Publish shared awareness or direct attention:

```bash
mosaico channel send --channel <channel> --message "..."
mosaico channel send --channel <channel> --tag <agent-ref> --message "..."
```

Start a new fabric session in the workspace that owns the work:

```bash
mosaico dispatch <agent-ref> --workspace <workspace> \
  --channel <channel> --message "..."
```

When progress truly depends on a response, use one bounded wait:

```bash
mosaico channel send --tag <agent-ref> --wait 600 --message "..."
mosaico wait 60 --channel <channel> --from <agent-ref>
```

Through MCP, set `wait_seconds` on `mosaico.channel_send` for the correlated
form, or call `mosaico.wait` with `timeout_seconds` plus optional `channels` and
`from` filters for the ambient form. Both return a successful timeout outcome
when the bound expires. A correlated send preserves its accepted send result
under `send` and returns the message-or-timeout outcome under `wait`.

For a distinct multi-participant workstream, read
[Channel Creation](channel-creation.md). When ownership or context crosses a
workspace boundary, read [Cross-Workspace Coordination](cross-workspace.md).

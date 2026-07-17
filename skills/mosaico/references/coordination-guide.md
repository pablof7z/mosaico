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
server. Use `--attach <label>=<file>` and include the exact `[<label>]` marker
in the message; the marker becomes the uploaded file's public URL. Repeat
`--attach` for multiple files. Labels may contain letters, digits, `.`, `-`,
and `_`.

Treat an attachment as shared outside the local filesystem. Do not attach
secrets, credentials, or private material without explicit authorization. If a
file is only local evidence, summarize it instead.

```bash
mosaico channel send --channel <channel> \
  --attach report=./report.pdf \
  --message "The review findings are in [report]."

mosaico channel reply <message-id> \
  --attach trace=./trace.json \
  --message "The reproducer trace is [trace]."
```

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

Inspect a message before responding:

```bash
mosaico channel read --id <message-id>
```

Acknowledge or continue its thread:

```bash
mosaico channel react <message-id> "👍"
mosaico channel reply <message-id> --message "..."
```

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

For a distinct multi-participant workstream, read
[Channel Creation](channel-creation.md). When ownership or context crosses a
workspace boundary, read [Cross-Workspace Coordination](cross-workspace.md).

# Cross-Workspace Coordination

Read this reference when a dependency, participant, decision, or artifact spans
workspaces.

## Treat Workspace As An Execution Boundary

A workspace gives related work a shared project context and channel root. A
session starts in one workspace while the fabric keeps its wider purpose and
relationships visible.

Coordinate across that boundary when another workspace owns relevant knowledge,
an affected artifact, an upstream or downstream dependency, or the participant
best placed to help. Join the relevant conversation, involve an existing agent
there, or launch an agent in that workspace.

Keep implementation ownership near the workspace that owns the affected
artifact. Carry consequential decisions, dependencies, and handoffs back to
every workspace whose work they change.

## Orient Before Acting

Injected deltas may already contain the relevant workspace, channels, agents,
and recent activity. Expand only when the decision needs broader current state:

```bash
tenex-edge my session
tenex-edge channel list --all-workspaces
tenex-edge channel list --workspace <workspace>
```

Join an existing cross-workspace channel using the channel ID returned by the
workspace listing, then use that joined channel for focused coordination:

```bash
tenex-edge channel join <channel-id>
tenex-edge channel send --channel <channel-id> --message "..."
```

Prefer an existing agent whose context and ownership match the request. For a
new independent session, dispatch the matching available fabric agent into the
workspace that owns the work and include the channel where coordination should
continue:

```bash
tenex-edge dispatch <agent-ref> --workspace <workspace> \
  --channel <workspace.channel> --message "..."
```

Keep active coordination in that focused channel and surface its consequences
in the relevant parent channels.

## Preserve The Relationship

Give cross-workspace coordination enough context to survive different local
assumptions:

- why the workspaces are related;
- which artifact, contract, or decision crosses the boundary;
- current evidence and authoritative source;
- what each side owns next;
- where milestones, blockers, and completion should be reported.

If the collaboration becomes an ongoing subtopic, use
[Channel Creation](channel-creation.md) to give it a durable home. Use the
[Coordination Guide](coordination-guide.md) for attention and handoff mechanics.

# Channel Creation

Read this reference when deciding where shared coordination belongs or when
creating, joining, switching, seeding, or reorganizing channels.

## Choose The Right Channel

Use the narrowest channel that owns the active conversation.

- Continue in the current channel when the work directly serves its topic and
  shares the same participants and decisions.
- Reuse an existing channel when its topic already owns the work.
- Join a channel when its ongoing context and directed messages should remain in
  your awareness.
- Switch to a channel when it becomes the main focus of your coordination.
- Create a channel as a distinct subtopic or workstream begins to need sustained
  back-and-forth, its own decisions, or continuity across participants and
  sessions.
- Split concurrent coordination streams into separate children while their
  parent remains the shared integration surface.

Create proactively. A focused channel gives the work a durable address, keeps
its working context coherent, and lets the relevant participants coordinate
closely while adjacent work stays legible.

## Place It In The Hierarchy

Create the channel beneath the closest parent that owns its broader outcome.
`tenex-edge channel create` uses the current active channel as that parent.

Use parent channels for broad awareness, integration, cross-cutting questions,
and updates that affect adjacent work. Use child channels for the detailed
working conversation. Nest a narrower stream beneath the child that owns it.

Choose a durable topic name and a short stable `--about` description. Treat the
name and description as shared orientation for future participants.

## Seed The Channel

Start the channel with enough context for another participant to act:

- objective and desired outcome;
- relevant background and current state;
- constraints and decisions already made;
- active dependencies or blockers;
- participants or capabilities that should contribute;
- expected next action or handoff.

## Work There And Surface Consequences

Keep active discussion, evidence, intermediate decisions, and coordination in
the focused channel. Publish milestones, decisions, dependencies, blockers,
completion, and handoffs in the parent when they change what its audience should
know or do. Summarize the consequence and point to the focused channel for
detail.

Keep bounded in-session helper work with the parent agent, then publish the
useful synthesis to the channel that owns the outcome.

## Commands

Inspect the available hierarchy:

```bash
tenex-edge channel list
tenex-edge channel list --workspace <workspace>
```

Join for passive context or switch the active coordination focus:

```bash
tenex-edge channel join <channel>
tenex-edge channel switch <channel>
```

Add a human or bring an existing session into a channel when its participation
is needed. Do not use `channel add` to start a new agent; use `dispatch` for
that.

```bash
tenex-edge channel add <pubkey-or-npub-or-nip05> <channel>
tenex-edge channel add --session <session-handle> <channel>
```

Create and focus a child beneath the current channel:

```bash
tenex-edge channel create <relative-path> --about "short stable description"
```

Maintain a channel's durable metadata only when you own that decision:

```bash
tenex-edge channel edit <channel> --about "revised stable description"
tenex-edge channel leave <channel>
```

`channel archive <channel>` marks the channel archived and removes every
non-admin member. Treat it as destructive: require explicit authority and post
or preserve any necessary handoff before using it.

`channel init` registers the current non-git directory as a workspace. Use it
only when the directory genuinely needs a durable workspace binding; do not use
it to create an ad hoc coordination room.

Send an update to a specific joined channel:

```bash
tenex-edge channel send --channel <channel> --message "..."
```

For channels in another workspace, read
[Cross-Workspace Coordination](cross-workspace.md) before acting.

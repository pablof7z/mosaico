# Channel RPCs

Companion to the [daemon RPC catalog](daemon-rpc-surface.md). This document owns
the channel-addressing, lifecycle, and membership contracts. Messaging RPCs are
documented in [daemon-rpc-messaging.md](daemon-rpc-messaging.md).

## Public channel references

Every agent- or operator-supplied channel reference is a full absolute path:

```text
/root
/root/child
/root/child/grandchild
```

The first segment names an existing root channel. Each later segment is an
exact, case-insensitive match for a child's kind:39000 `name` beneath the
previous segment. Paths resolve globally; they are not relative to the caller.

Bare names, relative paths, `#` aliases, `@` selectors, and opaque NIP-29 `h`
values are rejected at the public boundary. An unresolved path is an error and
never creates a channel.

Opaque `h` values remain internal protocol identifiers. Internal launch and
provider calls may carry them, but no agent-visible result, confirmation,
error, list entry, or injected context may expose one.

## Membership model

A session has one immutable launch workspace and one additive set of joined
channels containing zero or more entries. It has no current, active, focused,
or switched channel.

- `channel_join` adds exactly one existing channel.
- `channel_leave` removes exactly one joined channel, including the final one.
- `channel_create` creates one leaf and joins the creator to it without leaving
  any other channel.
- Ordinary stop, idle eviction, crash, and resume preserve the joined set.
- Only an explicit leave, revoke/forget, archive removal, or failed-admission
  cleanup removes membership.

`channel_join` also returns an optional `history_notice`. When the join is new
and the channel already has conversation, it summarizes only the newest
pre-join activity cluster (message count, duration, and a bounded author list)
and points to the explicit-read guidance. It never returns pre-join bodies.

## `root_channels`

```jsonc
params: {}
result: {"channels": [{"channel": "/root", "about": "…"}, …]}
```

Returns known root channels using public paths.

## `channel_edit`

```jsonc
params: {"channel": "/root/child", "about": "…"}
result: {"event_id": "hex", "channel": "/root/child", "about": "…",
         "confirmed": true}
```

Updates an existing channel's durable description.

## `channel_members`

```jsonc
params: {"channel": "/root/child"}
result: {"channel": "/root/child",
         "members": [{"pubkey": "hex", "slug": "agent", "role": "member"}, …]}
```

Returns the current relay-confirmed membership roster.

## `channel_add_member` / `channel_remove_member`

```jsonc
params: {"channel": "/root/child", "pubkey": "hex", "admin": false}
result: {"channel": "/root/child", "pubkey": "hex", "role": "member",
         "confirmed": true}
```

Adds or removes one human or agent pubkey. Membership mutations are confirmed
against relay state before success is reported.

## `channel_create`

```jsonc
params: {"name": "child", "about": "…",
         "parent_channel": "/root/parent", "agents": […]}
result: {"channel": "/root/parent/child", "joined": true,
         "orchestration_event_id": "hex"|""}
```

The parent must already exist. Creation mints exactly the named leaf; it never
creates missing ancestors. The creator joins the leaf additively. `joined`
describes that admission and never implies switching or leaving another
channel.

## `channel_list`

```jsonc
params: {"all": false, "recursive": false, "workspace": null}
result: {
  "sections": [{
    "kind": "own",
    "title": "Your workspace",
    "channels": [{
      "path": "/root",
      "about": "…",
      "agents": 2,
      "last_activity": "3 min ago",
      "children": []
    }]
  }]
}
```

Default mode gives the immutable launch workspace its own section, and expands
each root only when the caller has joined a channel beneath it. Roots with no
caller membership stay compact with their total descendant count. `recursive`
expands every root; `all` compactly lists every root; `workspace` expands one
named root. Agent counts exclude humans and management backends. Unknown
counts or activity are omitted rather than fabricated.

## `channel_join` / `channel_leave` / `channel_archive`

```jsonc
params: {"channel": "/root/child", "session": "npub1…"|"hex"|"handle"|null}
join result: {"channel": "/root/child",
              "history_notice": "…"|null}
leave result: {"channel": "/root/child", "left": true|false}
```

All three require an existing full path. Join is additive and never creates.
Leave is explicit and may leave the joined set empty. Archive updates metadata
and removes non-admin members after relay confirmation.

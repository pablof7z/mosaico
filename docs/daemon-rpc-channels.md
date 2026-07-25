# Channel RPCs

Companion to the [daemon RPC catalog](daemon-rpc-surface.md). This file owns the
channel addressing contract and the channel lifecycle/membership RPCs. The
messaging RPCs that also take a channel reference live in
[daemon-rpc-messaging.md](daemon-rpc-messaging.md).

## Channel references

Two different things are spelled `"channel"` on this wire, and they are not
interchangeable.

**Channel reference** — an operator/agent-supplied handle for a channel. Every
one of them REQUIRES either a full absolute path or an explicit id selector:

- `/<workspace>[/<child>…]` — segment 0 must match an existing top-level
  workspace channel's `channel_h` (a workspace root's `channel_h` IS its slug);
  every further segment is an EXACT, case-insensitive match against a child
  channel's kind:39000 `name` under the previous segment.
- `@<id-prefix>` — the channel whose opaque id starts with the prefix.

Everything else is rejected before any lookup with `channel must be a full path
starting with "/", e.g. /workspace/child`: bare names (`planning`), bare raw
`h` ids, relative paths, and suffix/partial paths. There is no fuzzy matching,
no caller-scoped root, and no exception for the caller's own channel.

Resolution is GLOBAL — any session may address any channel in any workspace —
and EXACT, so a well-formed full path names exactly one channel or none:

- **found** → the RPC proceeds against the resolved `channel_h`;
- **ambiguous** (only possible for `@<id-prefix>`) → the RPC succeeds with
  `{"ambiguous": ["@<id>", …], "reference": "<as supplied>"}` and does nothing
  else; re-run with one of the returned selectors;
- **not found** → error listing the workspace's actual channel paths (plus any
  other path segment that is itself a separate workspace). Nothing is created.

Reference params: `channel_edit.channel`, `channel_add_member.channel`,
`channel_join`/`channel_leave`/`channel_archive`.`channel`,
`channel_create.parent_channel`, `channel_send`/`channel_read`.`channel`,
`invite.channel`, and the `archive` management command's argument.

**Raw opaque id** — a `channel_h` the caller already holds. These params are
NOT references and take no path: `session_start.channel`, `pty_spawn.channel`
and `.root`, `channel_create.parent`, `channel_members.channel`,
`channel_remove_member.channel`, `channel_list.channel`, `tail.channel`, and
`channel_resolve.channel` (the literal parent for a name lookup).
`channel_wait.channels` is the one hybrid: it accepts a full path, an
`@<id-prefix>`, or a raw `h`, but matches only among channels the calling
session has already joined.

## `root_channels`
```jsonc
params: {}
result: {"channels": [ {slug, about}, … ]}
```
Returns all known workspace root channels from the daemon's cache.

## `channel_edit`
```jsonc
params: {"channel": "/workspace/child"|"@id-prefix", "about": "…"}
result: {"event_id": "hex", "channel": "channel-h", "about": "…", "confirmed": true}
      | {"ambiguous": ["@id", …], "reference": "…"}
```
Publishes an updated NIP-29 kind:39000 group metadata event for a channel.
`channel` is a channel reference; the channel must already exist.

## `channel_members`
```jsonc
params: {"channel": "channel-h"}
result: {"members": [ {pubkey, slug, role}, … ]}
```
Returns the current membership list for the given NIP-29 group. `channel` is a
raw opaque id, not a reference.

## `channel_add_member`
```jsonc
params: {"channel": "/workspace/child"|"@id-prefix", "pubkey": "hex", "admin": bool}
result: {"channel": "channel-h", "pubkey": "hex", "role": "member"|"admin", "confirmed": true}
      | {"ambiguous": ["@id", …], "reference": "…"}
```
Adds a human pubkey (hex, npub, or NIP-05) to a NIP-29 group. `channel` is a
channel reference resolved globally; the caller must still have a resolvable
session anchor or be invoked from a workspace directory.

## `channel_remove_member`
```jsonc
params: {"channel": "channel-h", "pubkey": "hex"}
result: {"ok": true}
```
Removes a pubkey from a NIP-29 group. `channel` is a raw opaque id.

## `channel_create`
```jsonc
params: {"name": "…", "about": "…", "parent": "channel-h"|null,
         "parent_channel": "/workspace/child"|"@id-prefix"|null, "agents": [...], ...}
result: {"child_h": "…", "display_path": "…", "switched": bool,
         "orchestration_event_id": "hex"|""}
      | {"ambiguous": ["@id", …], "reference": "…"}
```
Creates a child channel under `parent_channel` (a channel reference), else the
caller's current channel, else the literal `parent` opaque id, else the
`<workspace>` root resolved from cwd. `parent_channel` must already exist —
`channel_create` mints only the one leaf it was given a name for, never the
ancestor chain.

## `channel_list`
```jsonc
params: {"channel": "channel-h"}
result: {"channel": "…", "rooms": [ {child_h, name, about, depth}, … ]}
```
Lists the materialized child-channel tree under a channel. `channel` is a raw
opaque id.

## `channel_join` / `channel_leave` / `channel_archive`
```jsonc
params: {"channel": "/workspace/child"|"@id-prefix", "session": "npub1…"|"hex"|"handle"|null, ...}
result: {"channel": "channel-h", ...}
      | {"ambiguous": ["@id", …], "reference": "…"}
```
Mutates the caller session's channel membership or archives a channel.
`channel` is a channel reference. `channel_join` alone has `mkdir -p`
semantics: when the path names missing descendants of an EXISTING workspace it
creates the whole missing chain and joins the leaf. The workspace itself (path
segment 0) is never auto-created — an unknown workspace is a hard rejection.
`channel_leave` and `channel_archive` never create anything.

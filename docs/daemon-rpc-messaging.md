# Channel messaging RPCs

Companion to [daemon-rpc-channels.md](daemon-rpc-channels.md). Every public
channel value below is a full `/root[/child…]` path. No messaging surface
accepts or returns opaque protocol ids.

Explicit targets must already be in the caller's joined-channel set. Omitting a
target is valid only when that set contains exactly one channel. A session with
zero or multiple joined channels must name the destination.

## `channel_read`

```jsonc
params: {"id": "event-id"|null, "channel": "/root/child"|null,
         "since": u64|null, "limit": u64|null, "offset": u64,
         "tail": bool, "live": bool}
stream: {"item": {"event_id": "hex", "from_slug": "agent",
                  "channel": "/root/child", "body": "…",
                  "truncated": false, "created_at": 123}}
```

Normal history reads use the shared 100-word render limit and set
`truncated=true` when content is shortened. Exact `id` reads return the complete
message body. Explicit history reads are deliberate inspection and are not
subject to automatic-context join cutoffs.

## `channel_send`

```jsonc
params: {"message": "see [report]",
         "attachments": [{"label": "report", "path": "/absolute/report.pdf"}],
         "channel": "/root/child"|null, "long_message": false}
result: {"event_id": "hex", "channel": "/root/child",
         "mentioned_pubkeys": ["hex"], "mentioned_labels": ["agent"],
         "recipient_reminders": []}
```

Publishes a kind:9 event signed by the caller's session key and succeeds only
after checked relay acceptance. Destination selection never changes session
membership. Explicit p-tags to identities owned by this daemon are durably
parked under the exact recipient pubkey whether the executor is running,
stopped, route-less, or revoked. Locality selects the executor; it does not
decide whether the relay-accepted mention is valid. Remote p-tags cause no
local action. Untagged channel chat remains ambient awareness.

Attachment labels must appear as `[label]`. The daemon uploads and verifies each
blob before publishing, then replaces markers with public URLs. Invalid,
duplicate, unused, mismatched, or failed attachments abort without publishing.

## `channel_wait`

```jsonc
params: {"timeout_secs": 60, "channels": ["/root/child"],
         "from": "human-or-agent"|null, "reply_to": "event-id"|null}
result: {"outcome": "message", "waited_secs": 4,
         "channels": ["/root/child"],
         "message": {"event_id": "hex", "channel": "/root/child", "body": "…"}}
      | {"outcome": "timeout", "timeout_secs": 60,
         "channels": ["/root/child"]}
```

Wait captures a message-arrival cursor and the caller's joined-channel set
before subscribing. Repeated `channels` entries may only narrow that set. An
omitted list means all joined channels, including an empty set. `from` narrows
the author. A correlated send-wait additionally requires a native reply tag
pointing to the outbound event. Backend-management traffic and the caller's own
messages are excluded.

The CLI always renders the outcome through the canonical `<mosaico>` envelope.
Timeout is a successful RPC outcome.

## `channel_reply`

```jsonc
params: {"id": "event-id-or-prefix", "message": "see [report]",
         "attachments": [{"label": "report", "path": "/absolute/report.pdf"}],
         "long_message": false}
result: {"event_id": "hex", "reply_to": "hex",
         "channel": "/root/child", "mentioned_pubkey": "hex",
         "recipient_reminders": []}
```

Publishes a threaded NIP-10 reply in the original message's channel and targets
its author. The caller must belong to that channel. Attachment handling matches
`channel_send`.

## Automatic ambient cutoff

Automatic ambient context admits a message only when both conditions hold:

1. its local arrival sequence is later than the session's join watermark; and
2. its signed timestamp is at or after the recorded join time.

Missing or unverifiable evidence fails closed. This prevents future-dated
pre-join events and backdated post-join events from leaking old conversation
bodies. On a new join, Mosaico instead renders a compact recent-activity hint
with count, time window, and authors, plus a pointer to the coordination skill
for deliberate history navigation. Direct inbox rows do not use this cutoff:
they belong to an exact local recipient because the accepted event explicitly
p-tagged that pubkey, and remain claimable across route and runtime changes.

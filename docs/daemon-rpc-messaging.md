# Channel messaging RPCs

Companion to the [daemon RPC catalog](daemon-rpc-surface.md). This file owns the
channel messaging wire contracts.

## `channel_read` (streaming)

```jsonc
params: {"id": "event-id"|null, "channel": "…"|null, "since": u64|null,
         "limit": u64|null, "offset": u64, "tail": bool, "live": bool, ...}
stream: {"item": {event_id, from_pubkey, from_slug, channel, body,
                  truncated, created_at, ...}}
```

Streams channel chat from the relay-event cache. Normal history reads truncate
bodies past the fabric render limit and include `truncated=true`; exact
`--id`/`id` reads fetch one event by id and return the full body without channel
inference.

## `channel_send`

```jsonc
params: {"message": "…", "channel": "…"|null, "long_message": bool, ...}
result: {"event_id": "hex", "channel": "channel-h", "mentioned_pubkeys": ["hex", ...],
         "mentioned_labels": ["agent", ...]}
```

Publishes a NIP-29 kind:9 chat message signed by the caller's own per-session key
and returns only after checked relay acceptance. Messages over the fabric render
limit are rejected unless `long_message=true`. `channel` is destination targeting
only; caller identity is resolved independently from the session anchors.

## `channel_wait`

```jsonc
params: {"timeout_secs": 60, "channels": ["channel-ref", ...],
         "from": "human-or-agent"|null, "reply_to": "event-id"|null, ...}
result: {"outcome": "message", "waited_secs": 4, "channels": ["channel-ref", ...],
         "message": {event_id, from_pubkey, from_slug, channel, channel_ref, body, ...}}
      | {"outcome": "timeout", "timeout_secs": 60, "channels": ["channel-ref", ...]}
```

One blocking, agent-only read primitive backs both top-level `tenex-edge wait`
and `channel send --wait`. Ambient waits capture the exact caller session's
daemon-local message-arrival cursor and active-channel set before subscribing,
then return the first new visible kind:9 row. Repeated explicit channels narrow
that active set; `from` narrows the author.

Correlated send waits start at the outbound message cursor and require the
reply's native `e` tag to reference that event. Backend-management traffic and
the caller's own messages are excluded. Timeout is a successful RPC outcome.
The CLI renders both outcomes through the canonical `<tenex-edge>` agent
envelope and exposes no JSON/human mode.

## `channel_reply`

```jsonc
params: {"id": "event-id-or-prefix", "message": "…", "long_message": bool, ...}
result: {"event_id": "hex", "reply_to": "hex", "channel": "channel-h",
         "mentioned_pubkey": "hex"}
```

Publishes a threaded NIP-10 reply to an existing channel message. The daemon
resolves `id` against the channel read model, targets the original author's
pubkey, and signs the reply with the caller's per-session key.

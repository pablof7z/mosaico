---
title: Tenex-Edge Agent Identity
slug: tenex-edge-agent-identity
topic: tenex-edge
summary: Identity is per session — each session derives its own Nostr keypair from the machine's management key; trust is NIP-29 channel membership.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-07-09
verified: 2026-07-09
compiled-from: conversation
sources:
  - session:019f12ce-2569-72e0-b959-6d87d5daec5d
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
  - session:019f12f9-8a0b-7012-ad2f-f4d0cb035d2b
  - session:75f62bb9-f564-4633-8741-997dfea1d0e7
---

# Tenex-Edge Agent Identity

## Per-Session Identity

Identity is per session, not per agent. There is no durable per-agent keypair and
no ordinals. Every session mints its own keypair at start as
`derive(management_secret, session_id)`, where the machine's management key
(`tenexPrivateKey`) is the only secret stored on the machine. Because a session's
key is always re-derivable from `(management_secret, session_id)`, sessions are
recoverable and resumable without ever storing an nsec.

`<edge_home>/agents/<slug>.json` is role config — harness, provider, model — not
an identity. Launching a role produces a fresh session with a fresh derived key;
the role file contributes behavior, not a signing identity.

## Agent/Session Handle

Each session publishes a kind:0 profile whose `name` is `@<agent-slug>/<session-id>`.
For example, a Codex session can be mentioned as `@codex/echo123`. That
`@agent/session` handle is the p-taggable mention target peers use to address the
session. Since the handle contains the canonical session id, a resumed session
keeps its handle.

## Trust Is Channel Membership

Trust is NIP-29 channel membership, exclusively. The machine's management key adds
a session's pubkey as a member of a channel; a session is removed from membership
on clean end and after 10 minutes with no heartbeat (TTL prune). An expired
session still appears in `who` history and remains re-derivable and resumable —
membership is presence, not the definition of the session's identity.

## Roster vs. Members

The roster (`available-agents`) is the set of role configs on the machine — the
*types* you can add to a channel. Channel *members* are concrete sessions,
rendered as their role plus `@agent/session`. Adding a role to a channel spawns a
new session; that session is what becomes a member.

## Session Identification and Routing

The raw `session_id` is the internal correlation id, and the derived pubkey is
what signs and is routed to. Peers reference a session by its `@agent/session`
handle, never by raw pubkey. A mention that cannot be resolved to a
current member is silently treated as no-mention rather than erroring, so mention
resolution never blocks chat delivery.

## Session Resume

Resume resolves sessions by exact raw `session_id`, then by `session_id` prefix.
Because the signing key and the handle are both derived from the session id,
resuming a session reconstitutes the same identity without any stored
secret beyond the machine's management key.

## Identity Commands

`tenex-edge who`, run inside an agent session, shows the caller who they are and
which channel they are on: a self header of the form
"You are **@agent/session** on **{channel}**." followed by pubkey, status,
membership, and pending counts. The self header is prepended to both `who` and the
`who --live` fabric view. The `(you)` member match keys on the session's derived
pubkey.

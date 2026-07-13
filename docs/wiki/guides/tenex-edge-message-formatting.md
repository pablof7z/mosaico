---
title: Tenex-Edge Message Formatting
slug: tenex-edge-message-formatting
topic: tenex-edge
summary: "The @mention is a session-targeted Nostr kind:9 event with a p-tag addressed to another agent's pubkey that gets server-side-routed into the target session's in"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-07-13
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:d39d3357-06d0-418a-bdbe-f288a9f9670f
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
  - session:bdb6c341-4dd4-48e7-9764-e80242beb005
  - session:75f62bb9-f564-4633-8741-997dfea1d0e7
  - session:e0eba763-d227-40ca-a9d2-aaad5b192130
  - session:fea5307b-d9a0-46fe-977c-408e5e0e0ff4
  - session:a62822c5-d09c-4a83-9251-a3856d276ac4
---

# Tenex-Edge Message Formatting

## Direct Mentions

The @mention is a session-targeted Nostr kind:9 event with a p-tag addressed to another agent's pubkey that gets server-side-routed into the target session's inbox and injected as a literal conversational turn into the target's live PTY session.

When the sender is a whitelisted pubkey (human) and the agent is in a PTY-hosted session, a direct mention is pasted as a bare turn: `<@pablo> @developer hey there`.

When the sender is not a whitelisted pubkey (i.e. an agent) in a PTY-hosted session, a direct mention is pasted as `[tenex-edge mention] <@agent1> Hello @developer`.

Agent-to-agent mentions in pty are pasted as real turns and auto-publish replies, with no gating or suppression. A soft consecutive-auto-turn depth limit is reserved as a future safety valve but is not implemented. When agent1 (launched via tenex-edge launch) mentions agent2 (also launched via tenex-edge launch), the mention is immediately injected into agent2's session as a user message attributed to agent1. Injected mentions appear in the target agent's context via the userPrompt hook. When agent2 replies back mentioning agent1, and agent1 was launched as a raw `claude` session (not via tenex-edge launch), the reply is not auto-injected into agent1's session — the user must manually ask agent1 whether it received the message.

In a hooks-only session, a direct mention is rendered inside a `<tenex-edge>` wrapper with a reply CLI hint and no message id. No system path auto-publishes kind:9 chat events; publishing happens only via explicit `tenex-edge channel send` by an agent or a user. The `user-prompt-submit` hook does not mirror the user's prompt as a kind:9 chat event (the `rpc_user_prompt` auto-publish path is removed). The agent Stop hook does not auto-publish the agent's turn output as a kind:9 chat event (the `publish_agent_reply` auto-publish path is removed).

When a hooks-only turn has both a direct mention and background chatter, the mention block and the activity block are emitted as two separate `<tenex-edge>` blocks — mention first, then activity — rather than merged.

When a mention is brought into an agent's attention via any injection path (PTY-hosted or hook-only), an explicit instruction is included reminding the agent to respond via `tenex-edge channel send`. The terminal mention envelope produced by `render_terminal_mention` includes this reply instruction. The hook-only `[MENTIONS YOU]` mention wrapper produced by `render_messages` includes this reply instruction.

Mention tokens in message bodies are normalized to `@<name>` display form by `rewrite_body_mentions`, the single source-of-truth resolver. It scans text for `nostr:npub…`/`nostr:nprofile…` tokens, decodes them, looks up the profile, and replaces each token with `@<name>` (falling back to `pubkey_short`). The `channel read` path runs this resolver before rendering.

Whitelisted human operators (checked against config `whitelistedPubkeys` via `is_whitelisted`) who have no session or host are rendered with a bare `<@name>` (no host segment) instead of `<name@?>` in both `channel read` CLI output and the fabric_context snapshot path — consistent with the bare rendering already used in terminal-injected mention rendering.

Chat mentions use the session's dashed kind:0 profile name (for example, `@codex-quill-peak-369`). Unresolvable mention tokens are silently treated as no-mention rather than blocking chat delivery. Mention resolution reverse-looks-up `relay_profiles` by that public handle.

The channel-send confirmation line names the dashed mentioned handle returned by the RPC, falling back to plain `sent chat {id}` when no mention is present.

`tenex-edge channel send` refuses messages longer than 600 characters, erroring out and offering `--long-message` for longer messages.

`tenex-edge channel send --wait <seconds>` keeps the command open until an
explicit kind:9 reply references the sent event. Unrelated channel chatter does
not complete it; when the send tagged recipients, only replies authored by those
targets qualify. A successful wait renders the same `<tenex-edge><channel><message>`
envelope used for direct delivery.

`tenex-edge wait <seconds>` blocks for the next visible kind:9 chat event. With
no `--channel`, it snapshots every channel the exact calling session is active
on; repeated `--channel` flags narrow the set and `--from` narrows the author.
Backend-management traffic and the caller's own messages never qualify. Timeout
is a normal exit-0 `<tenex-edge><wait outcome="timeout" ...>` envelope. These
agent-only waits have no JSON or human-table rendering mode.

@-mentioning someone from a subchannel they are not in is a cross-channel mention using their dashed session handle, with no membership side-effects; replying or joining requires an explicit `channel add` or `channel switch`.

<!-- citations: [^bdb6c-1833e] [^d39d3-7d6ac] [^bd868-1c088] [^bd868-dce28] [^bd868-f7785] [^75f62-ebb61] [^e0eba-b9cc1] [^e0eba-5f8a4] [^e0eba-7764c] [^fea53-85a33] [^a6282-aa4e7] -->
## Ambient Chatter

Ambient/background chatter is rendered inside a `<tenex-edge>` wrapper as a timeline with `<@name - Xm ago>` prefixes, identical for pty and hooks sessions, with no reply hint.

Ambient chatter is never pasted into a PTY session (it would force the agent to answer things it wasn't asked); it is always surfaced through hook context as a framed FYI block.

The relative-time suffix in ambient chatter is shown only when the message is older than ~2 minutes, so fresh lines stay clean. <!-- [^d39d3-c3568] -->

## Envelope Format and Message IDs

The `(message id: …)` line is dropped from all envelope formats. <!-- [^d39d3-c1f19] -->

## Echo Suppression and Injection Marking

The `FABRIC_INJECTION_MARKER` constant `[tenex-edge]` is the prefix used on injected envelopes to prevent echo loops — the daemon checks `prompt.trim_start().starts_with(FABRIC_INJECTION_MARKER)` to skip re-publishing already-injected text.

Echo suppression uses explicit inbox ledger states. When pty pastes delivered mention rows as a prompt, those rows become `injected`; the later `user-prompt-submit` hook consumes the matching rendered event group into `echo_consumed` instead of relying on a short-lived text hash. <!-- [^d39d3-39962] -->

## Session Identity and Display

The `who` command, when run inside an exact agent session, emits XML with a `<self>` row, global available-agent capabilities, and workspace/channel membership. Concurrent sessions render directly by distinct dashed handles. Session-start and hook echo responses carry only the canonical `session_id` when they need an internal correlation handle. <!-- [^bd868-e816c] -->

## Backend Management Traffic

Management commands (`list agents`, `list sessions`, `add agent`, `kill`, `archive`) are ordinary kind:9 NIP-29 chat events p-tagging the daemon's backend key, with replies published as real broadcast chat events into the same shared group and cached in the same `messages`/`relay_events` tables as normal chat.

The `channel read` path filters out backend-authored or backend-p-tagged rows (both initial batch and `--live` tail) using the `is_backend_pubkey` check, so mgmt-command round-trips like `list agents` are not visible to other agents reading the channel.

The `is_backend_traffic` filter excludes chat events whose author or any p-tag recipient is the daemon's backend mgmt key or a pubkey flagged `is_backend`, protecting the hook-injected awareness snapshot from leaking mgmt exchanges. <!-- [^a6282-fb95f] -->

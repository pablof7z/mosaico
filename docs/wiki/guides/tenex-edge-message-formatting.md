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
updated: 2026-07-03
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:d39d3357-06d0-418a-bdbe-f288a9f9670f
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
  - session:bdb6c341-4dd4-48e7-9764-e80242beb005
  - session:75f62bb9-f564-4633-8741-997dfea1d0e7
  - session:e0eba763-d227-40ca-a9d2-aaad5b192130
  - session:fea5307b-d9a0-46fe-977c-408e5e0e0ff4
---

# Tenex-Edge Message Formatting

## Direct Mentions

The @mention is a session-targeted Nostr kind:9 event with a p-tag addressed to another agent's pubkey that gets server-side-routed into the target session's inbox and injected as a literal conversational turn into the target's live PTY session.

When the sender is a whitelisted pubkey (human) and the agent is in a PTY-hosted session, a direct mention is pasted as a bare turn: `<@pablo> @developer hey there`.

When the sender is not a whitelisted pubkey (i.e. an agent) in a PTY-hosted session, a direct mention is pasted as `[tenex-edge mention] <@agent1> Hello @developer`.

Agent-to-agent mentions in pty are pasted as real turns and auto-publish replies, with no gating or suppression. A soft consecutive-auto-turn depth limit is reserved as a future safety valve but is not implemented. When agent1 (launched via tenex-edge launch) mentions agent2 (also launched via tenex-edge launch), the mention is immediately injected into agent2's session as a user message attributed to agent1. Injected mentions appear in the target agent's context via the userPrompt hook. When agent2 replies back mentioning agent1, and agent1 was launched as a raw `claude` session (not via tenex-edge launch), the reply is not auto-injected into agent1's session — the user must manually ask agent1 whether it received the message.

In a hooks-only session, a direct mention is rendered inside a `<tenex-edge>` wrapper with a reply CLI hint and no message id. No system path auto-publishes kind:9 chat events; publishing happens only via explicit `tenex-edge chat write` by an agent or a user. The `user-prompt-submit` hook does not mirror the user's prompt as a kind:9 chat event (the `rpc_user_prompt` auto-publish path is removed). The agent Stop hook does not auto-publish the agent's turn output as a kind:9 chat event (the `publish_agent_reply` auto-publish path is removed).

When a hooks-only turn has both a direct mention and background chatter, the mention block and the activity block are emitted as two separate `<tenex-edge>` blocks — mention first, then activity — rather than merged.

When a mention is brought into an agent's attention via any injection path (PTY-hosted or hook-only), an explicit instruction is included reminding the agent to respond via `tenex-edge chat write`. The terminal mention envelope produced by `render_terminal_mention` includes this reply instruction. The hook-only `[MENTIONS YOU]` mention wrapper produced by `render_messages` includes this reply instruction.

Chat mentions use `@<agent-instance-label>` (e.g. @haiku, @haiku1) instead of `@<codename>`. The `extract_mentions` tokenizer accepts any agent-slug-shaped token matching `[A-Za-z0-9._-]+` optionally host-qualified as `label@host`, not only NATO-codename-shaped tokens. Unresolvable mention tokens are silently treated as no-mention rather than blocking chat delivery. `resolve_agent_pubkey(slug, host)` is a Store function that reverse-looks-up relay_profiles by slug+host to return the selected pubkey for an agent-instance label. The obsolete concrete-session lookup (`find_session_by_codename`) and the bail requiring a mention to name a concrete session codename are removed from `chat_write.rs`.

The chat-write confirmation line reads `mentioning @{label}` instead of `mentioning session {codename}`, driven by the RPC's `mentioned_label`, falling back to plain `sent chat {id}` when no mention is present. README.md chat-write documentation references `@<agent-label>` targeting.

`tenex-edge chat write` refuses messages longer than 600 characters, erroring out and offering `--long-message` for longer messages.

@-mentioning someone from a subchannel they are not in is a cross-channel mention using existing `@slug@backend` addressing, with no membership side-effects from mentions; replying or joining requires an explicit invite or switch.

<!-- citations: [^bdb6c-1833e] [^d39d3-7d6ac] [^bd868-1c088] [^bd868-dce28] [^bd868-f7785] [^75f62-ebb61] [^e0eba-b9cc1] [^e0eba-5f8a4] [^e0eba-7764c] [^fea53-85a33] -->
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

The `who` command, when run inside an agent session, displays a self-identity header (label, channel, host, pubkey, status, member, pending), so the roster command also answers "who am I here?". The self-header reads `You are **{label}** on **{channel}** ({host}).` with pubkey, status, member, and pending info, and never shows a transient alias or raw session id. The legacy duplicate-name disambiguation apparatus (`display_row_agent_name`, `agent_name_counts_for_scope`, `agent_count_key`) is deleted from `who` renderers; `WhoRow.slug` (carrying the ordinal label post-#98) renders directly. Session-start and hook echo responses carry only the canonical `session_id` when they need an internal correlation handle. `session_label` returns `agent@backend-label` (degrading to raw `session_id` only when slug is empty). <!-- [^bd868-e816c] -->

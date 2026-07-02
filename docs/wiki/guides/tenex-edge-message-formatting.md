---
title: Tenex-Edge Message Formatting
slug: tenex-edge-message-formatting
topic: tenex-edge
summary: "When the sender is a whitelisted pubkey (human) and the agent is in a tmux-wrapped session, a direct mention is pasted as a bare turn: `<@pablo> @developer hey"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:d39d3357-06d0-418a-bdbe-f288a9f9670f
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
---

# Tenex-Edge Message Formatting

## Direct Mentions

When the sender is a whitelisted pubkey (human) and the agent is in a tmux-wrapped session, a direct mention is pasted as a bare turn: `<@pablo> @developer hey there`.

When the sender is not a whitelisted pubkey (i.e. an agent) in a tmux-wrapped session, a direct mention is pasted as `[tenex-edge mention] <@agent1> Hello @developer`.

Agent-to-agent mentions in tmux are pasted as real turns and auto-publish replies, with no gating or suppression. A soft consecutive-auto-turn depth limit is reserved as a future safety valve but is not implemented.

In a hooks-only session, a direct mention is rendered inside a `<tenex-edge>` wrapper with a reply CLI hint and no message id.

When a hooks-only turn has both a direct mention and background chatter, the mention block and the activity block are emitted as two separate `<tenex-edge>` blocks — mention first, then activity — rather than merged.

Chat mentions use `@<agent-instance-label>` (e.g. @haiku, @haiku1) instead of `@<codename>`. The `extract_mentions` tokenizer accepts any agent-slug-shaped token matching `[A-Za-z0-9._-]+` optionally host-qualified as `label@host`, not only NATO-codename-shaped tokens. Unresolvable mention tokens are silently treated as no-mention rather than blocking chat delivery. `resolve_agent_pubkey(slug, host)` is a Store function that reverse-looks-up relay_profiles by slug+host to return the selected pubkey for an agent-instance label. The obsolete concrete-session lookup (`find_session_by_codename`) and the bail requiring a mention to name a concrete session codename are removed from `chat_write.rs`.

The chat-write confirmation line reads `mentioning @{label}` instead of `mentioning session {codename}`, driven by the RPC's `mentioned_label`, falling back to plain `sent chat {id}` when no mention is present. README.md chat-write documentation references `@<agent-label>` targeting.

<!-- citations: [^d39d3-7d6ac] [^bd868-1c088] [^bd868-dce28] [^bd868-f7785] -->
## Ambient Chatter

Ambient/background chatter is rendered inside a `<tenex-edge>` wrapper as a timeline with `<@name - Xm ago>` prefixes, identical for tmux and hooks sessions, with no reply hint.

Ambient chatter is never pasted into a tmux pane (it would force the agent to answer things it wasn't asked); it is always surfaced through hook context as a framed FYI block.

The relative-time suffix in ambient chatter is shown only when the message is older than ~2 minutes, so fresh lines stay clean. <!-- [^d39d3-c3568] -->

## Envelope Format and Message IDs

The `(message id: …)` line is dropped from all envelope formats. <!-- [^d39d3-c1f19] -->

## Echo Suppression and Injection Marking

The `FABRIC_INJECTION_MARKER` constant `[tenex-edge]` is the prefix used on injected envelopes to prevent echo loops — the daemon checks `prompt.trim_start().starts_with(FABRIC_INJECTION_MARKER)` to skip re-publishing already-injected text.

Echo suppression uses explicit inbox ledger states. When tmux pastes delivered mention rows as a prompt, those rows become `injected`; the later `user-prompt-submit` hook consumes the matching rendered event group into `echo_consumed` instead of relying on a short-lived text hash. <!-- [^d39d3-39962] -->

## Session Identity and Display

The `who` command, when run inside an agent session, displays a self-identity header (label, channel, host, pubkey, status, member, pending), so the roster command also answers "who am I here?". The self-header reads `You are **{label}** on **{channel}** ({host}).` with pubkey, status, member, and pending info, and never shows a transient alias or raw session id. The legacy duplicate-name disambiguation apparatus (`display_row_agent_name`, `agent_name_counts_for_scope`, `agent_count_key`) is deleted from `who` renderers; `WhoRow.slug` (carrying the ordinal label post-#98) renders directly. Session-start and hook echo responses carry only the canonical `session_id` when they need an internal correlation handle. `session_label` returns `agent@backend-label` (degrading to raw `session_id` only when slug is empty). <!-- [^bd868-e816c] -->

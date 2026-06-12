---
title: Tenex-Edge User Prompt Submit
slug: tenex-edge-user-prompt-submit
topic: tenex-edge
summary: "The user-prompt-submit hook creates a kind:1 Nostr note that is a root event (OP) with no e-tag"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-10
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:4d8b9567-a1dc-4836-86d9-20904df30c26
  - session:40a4d401-2520-4781-b747-b0ef19594bed
---

# Tenex-Edge User Prompt Submit

## User Prompt Submit Hook

The user-prompt-submit hook arm in cli.rs extracts the prompt field from stdin JSON, clones the session id, calls the user_prompt daemon RPC after turn_start, and fails open (eprintln + Ok(()) on any error rather than propagating). The UserPromptSubmit hook creates a kind:1 OP (root event with no e-tag) signed by the nsec stored in ~/.tenex/config.json under the userNsec key and publishes it to the NIP-29 group (h tag with project slug) with a p-tag referencing the agent pubkey that will process the message.

<!-- citations: [^98f99-7] [^98f99-18] [^98f99-23] -->
## Config Structure

The Config struct includes user_nsec: Option<String>, deserialized from the JSON field 'userNsec' via serde rename.

<!-- citations: [^98f99-8] [^98f99-24] -->
## Daemon Handler & Event Semantics

The rpc_user_prompt daemon handler resolves the session to obtain agent_pubkey and project, parses userNsec from config, and uses it to sign kind:1 events when a user submits a prompt from the CLI. Both rpc_user_prompt and rpc_project_edit are gated on userNsec being set in ~/.tenex/config.json. The handler builds a kind:1 OP with h (project), p (agent_pubkey), and session-id tags, and publishes signed via the daemon's shared transport. The session-id tag scopes user prompt events to the intended session, preventing fan-out to all sessions of the agent. No 'agent' tag should ever exist on kind:1 events — the system must not read or write the ['agent', pubkey, slug] tag. After the event is published, suppress_inbox_event pre-inserts it as delivered=1 so that relay echoes of the user's own prompt never appear as unread inbox items. The user_prompt path fails open when userNsec is absent or the relay is unreachable, so the hook does not block the editor. Activity vs Mention disambiguation on kind:1 events uses the presence of a p tag: events with a p tag are Mentions; events without a p tag are Activity. Codec disambiguation for kind:1 events follows priority order: p-tag → Mention, e-tag with 'root' marker → TurnReply, neither → Activity. The sender slug is not carried on the wire in kind:1 events; it must be resolved from the profile store at routing time when from_slug is empty. fetch_mentions_into_inbox must use an operator-key check (not an agent tag) to skip user-prompt events authored by the operator. handle_incoming must gate Mention routing with the same owner-key check so that operator-authored echoes from the relay never enter the inbox. When an agent finishes producing text (stop hook), it publishes a kind:1 TurnReply with its own key, e-tagging the root event and the triggering user prompt event. TurnReply kind:1 events carry two e-tags: one with 'root' marker pointing to the first user prompt of the session, and one with 'reply' marker pointing to the user prompt that triggered the current turn. The first user prompt in a session is the thread root (no e-tags), the agent's first reply e-tags both root and reply to that same root event, and subsequent user prompts carry root and reply markers referencing prior messages. Subsequent user prompts after the root must include e-tags with a root marker pointing to the thread root event and a reply marker pointing to the last agent TurnReply event. The TurnReply event ID must be persisted after publishing so that subsequent user prompts can reference it as the reply e-tag. When an artifact such as a kind:30023 event is published, it must e-tag the root conversation event to link back to the originating conversation.

<!-- citations: [^40a4d-13] [^98f99-9] [^4d8b9-1] [^40a4d-2] [^40a4d-3] [^40a4d-14] [^40a4d-17] -->

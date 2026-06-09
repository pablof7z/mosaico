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
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
---

# Tenex-Edge User Prompt Submit

## User Prompt Submit Hook

The user-prompt-submit hook arm in cli.rs extracts the prompt field from stdin JSON, clones the session id, calls the user_prompt daemon RPC after turn_start, and fails open (eprintln + Ok(()) on any error rather than propagating). The UserPromptSubmit hook creates a kind:1 OP (root event with no e-tag) signed by the nsec stored in ~/.tenex/config.json under the userNsec key and publishes it to the NIP-29 group (h tag with project slug) with a p-tag referencing the agent pubkey that will process the message.

<!-- citations: [^98f99-7] [^98f99-18] [^98f99-23] -->
## Config Structure

The Config struct includes user_nsec: Option<String>, deserialized from the JSON field 'userNsec' via serde rename.

<!-- citations: [^98f99-8] [^98f99-24] -->
## Daemon Handler & Event Semantics

The rpc_user_prompt daemon handler resolves the session to obtain agent_pubkey and project, parses userNsec from config, builds a kind:1 OP with h (project) and p (agent_pubkey) tags, and publishes signed via the daemon's shared transport. A kind:1 event with a p tag but no agent tag decodes as Activity (not Mention), so it does not land in any agent's inbox. <!-- [^98f99-9] -->

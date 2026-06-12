---
title: Tenex-Edge Session Display
slug: tenex-edge-session-display
topic: tenex-edge
summary: Session display IDs use a hash-based short code derived from the full UUID rather than truncating the UUID prefix.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-10
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:435ec383-d607-459b-a712-a00ed4decaa7
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
---

# Tenex-Edge Session Display

## Session Display IDs

Session IDs are displayed using a hash-based short code (session_short_code) rather than a UUID-based prefix (short_id), ensuring consistency across tail, who, and send-message commands. SessionId is a newtype in src/util.rs whose Display impl outputs the hash-based session_short_code, making format strings with {session_id} automatically correct and compile-time enforcing against misuse of pubkey_short for session IDs. short_id is renamed to pubkey_short everywhere, making it explicit that the function is only for public keys and preventing accidental use on session IDs. SessionId is used for Presence.session_id, Mention.target_session, Mention.from_session, and WhoRow.session_id; at the DB boundary, SessionId converts to/from String using .as_str() for writes and SessionId::from(s) for reads, and the domain layer uses the SessionId newtype throughout.

<!-- citations: [^435ec-3] [^ab999-4] [^ab999-41] [^56f9f-3] [^56f9f-4] [^56f9f-8] [^56f9f-12] [^56f9f-14] -->
## Thread Display in CLI

The `threads --project` CLI must print the full thread id (not a truncated `short_id`) so it can be used with `--thread`. <!-- [^ab999-42] -->

## RPC Thread Meta Response

`rpc_thread_meta` must not return bare JSON `null` for a missing thread; it must return an empty object so the client interprets it as a valid empty result rather than 'neither ok nor error'. <!-- [^ab999-43] -->

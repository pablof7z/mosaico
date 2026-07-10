---
title: Tenex-Edge Agent Display Handles
slug: tenex-edge-agent-display-handles
topic: tenex-edge
summary: The `agent/session` display handle uses the friendly codename (e.g
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-10
updated: 2026-07-10
verified: 2026-07-10
compiled-from: conversation
sources:
  - session:af454e46-7c4f-4182-ab2b-ebc50b1eb9ad
---

# Tenex-Edge Agent Display Handles

## Display Handle Definition

The `agent/session` display handle uses the friendly codename (e.g. `lark-summit-042`), not the raw internal `session_id` (e.g. `te-18c0e6e30d9c3c80-0`). <!-- [^af454-69c80] -->

`SessionIdentity::display_slug()` builds `agent/session` using `self.codename` instead of `self.session_id`. The `display_slug()` field is the root source used for kind:0 name, chat From labels, statusline, who, invite wait, and pty_rpc rendering. <!-- [^af454-5b76e] -->

## Rendering Call Sites

Peer/member rendering via `fabric_context/refs.rs::session_ref()` builds `agent/friendly_short_code(session_id)` instead of `agent/session_id`. <!-- [^af454-4ab54] -->

`who_snapshot/dormant.rs` and `cli/who/render/expired.rs` render the codename field instead of the raw `session_id`. <!-- [^af454-c9ca2] -->

The three daemon RPC/resolve call sites (`invite_rpc/resolve.rs`, `management_command/sessions.rs`, `rpc/agents.rs`) reconstruct handles using `friendly_short_code`. <!-- [^af454-7caef] -->

## Handle Resolution

Mention resolution accepts codename, short code, raw id, or prefix as valid handle forms. <!-- [^af454-a3535] -->

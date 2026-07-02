---
title: Tenex-Edge Agent Identity
slug: tenex-edge-agent-identity
topic: tenex-edge
summary: Agent-instance identity is modeled as one first-class object carried from session birth through all downstream consumers, so that no callsite recomputes which p
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:019f12ce-2569-72e0-b959-6d87d5daec5d
  - session:bd8689c8-4a5f-45b3-9dbe-758baec2a2f4
  - session:019f12f9-8a0b-7012-ad2f-f4d0cb035d2b
---

# Tenex-Edge Agent Identity

## Agent-Instance Identity

Agent-instance identity is modeled as one first-class object carried from session birth through all downstream consumers, so that no callsite recomputes which pubkey signs or which label to display. AgentInstanceIdentity is the authoritative identity struct that carries base_slug, display_slug, base_pubkey, pubkey, ordinal, and signing_keys for a session; it encapsulates the base-vs-ordinal identity policy, is created at session birth, threaded through EngineParams, and projected from the store for read-side callers. AgentInstanceIdentity provides display_slug(), agent_ref(), and signing_keys(&base_keys) as the only methods for base-vs-ordinal identity policy; display_slug() returns the ordinal label (e.g. 'haiku' for ordinal 0, 'haiku1' for ordinal 1). EngineParams is the runtime parameter struct that carries the AgentInstanceIdentity plus base keys through the engine spine, replacing the prior five parallel slug/pubkey/key fields it previously held. Store::instance_identity_for_session() and DaemonState::session_instance(&Session) are the projection helpers that read-side callers use to obtain the authoritative AgentInstanceIdentity, with safe fallback; signer integration tests assert agent identity through this durable store projection (backing `who`), not through the removed `whoami` RPC. Every publisher, renderer, and router (status, chat, profile, who, statusline, heartbeat, proposal, mention routing) consumes the AgentInstanceIdentity rather than reconstructing identity policy at edge sites. Each agent session signs its own kind:0 profile event with its own key: the ordinal key when it exists, the base key for ordinal 0, never the base key for all sessions. Two concurrent sessions of the same base agent publish distinct, self-consistent identities (claude for ordinal 0 and claude1 for ordinal 1, each with its own pubkey) on kind:0 and `who`, verified by the concurrent_same_agent_sessions_publish_consistent_identities integration test. session_codename generation is deleted entirely as a product concept; the codename apparatus (session_codename(...) removed from production paths, resume_by_codename, codename fields in hook/session-start JSON, codename disambiguation in who renderers) has been removed. SessionId's Display impl stops formatting as a codename, rendering the raw session_id instead; raw session_id is used only internally. (Previously: the ad-hoc keys_for_session(..).unwrap_or(base) and base-slug-with-ordinal-pubkey edge policy was used instead of AgentInstanceIdentity; a session_codename generator produced NATO-word + 4-digit hash labels like bravo4217 as an additional identity surface, now deleted.)

<!-- citations: [^019f1-3d443] [^bd868-ed09a] [^bd868-323a9] [^bd868-4f5d3] [^bd868-2ae6c] [^019f1-124aa] -->
## Naming Conventions

Agent names use the format agentName{ordinal} (e.g., 'haiku1'). All displays use the agentName{index} format instead of codenames. Duplicate agent names never exist in the same channel, because ordinal (cardinal) instance labels disambiguate them. The ordinal instance label (e.g. haiku1) is the primary same-agent disambiguator on the wire.

<!-- citations: [^019f1-d73ec] [^bd868-96ae0] [^019f1-d1a7b] -->
## Session Identification

Local sessions are p-tagged by their ordinal agent-instance label (e.g., 'haiku1'), not by codename. Raw session_id is the only internal correlation id. SessionId's Display impl renders the raw session_id, not a codename. Codename generation is fully retired: SessionId display stops formatting as a codename, and raw session_id is used only internally.

<!-- citations: [^019f1-10a4e] [^bd868-d7a8c] [^bd868-55c14] [^bd868-02d72] [^019f1-85d00] -->
## Identity Commands

The whoami command does not exist; instead, tenex-edge who shows the current agent who they are and what channel they are on when run inside an agent session context. The whoami CLI subcommand and its daemon dispatch are removed; its self-identity output is folded into the tenex-edge who command's agent-context output. When run by an agent, who displays a self header: 'You are **{label}** on **{channel}** ({host}).' followed by pubkey, status, member, and pending. The who self block carries label, pubkey, channel, host, is_member, working, status, pending, created_at, and a raw session_id — no codename field. The who self-header is prepended to both who output and who_live live fabric screens. The who fabric block's (you) member match keys on the ordinal-selected pubkey via instance.pubkey rather than rec.agent_pubkey. The codename disambiguation apparatus in who renderers (display_row_agent_name, agent_name_counts_for_scope, agent_count_key) is deleted; concurrent same-agent instances render their ordinal slugs directly (e.g. haiku/haiku1).

<!-- citations: [^019f1-f1f53] [^bd868-16873] [^bd868-6a26a] [^bd868-9b5dc] [^bd868-d2172] [^019f1-5dabb] -->
## Chat Mention Resolution

Chat mention extraction in extract_mentions accepts agent-slug-shaped tokens ([A-Za-z0-9._-]+, optionally @host-qualified). `@<agent-label>` targeting (e.g. @haiku / @haiku1) resolves to the selected instance pubkey. resolve_agent_pubkey resolves an agent-instance label (e.g. haiku1) to the ordinal-selected pubkey by reverse-looking-up relay_profiles by slug and host. Mention resolution never blocks chat delivery: an unresolvable mention token is silently treated as no-mention rather than erroring. The chat write CLI confirmation line displays 'mentioning @{label}' using the resolved agent label, falling back to plain 'sent chat {id}' when there is no mention.

<!-- citations: [^bd868-17c6e] [^bd868-b43e5] -->
## Session Resume

tmux resume resolves sessions by exact raw session_id then session_id prefix only. resume_by_codename is deleted. The Resume CLI help text reads 'Session id (prefix) to resume.' The HookTail CLI help text reads 'Filter panes/events to a session id (or prefix)'. The codename field is dropped from session_start and hook JSON responses.

<!-- citations: [^bd868-7dcbb] [^bd868-eafc6] [^bd868-a51fb] -->
## Operator Surface and Logging

The TUI session row tag surfaces the raw session_id or agent-instance label. Tail session tags use the raw session_id for operator correlation. The slog per-session debug log filename uses the raw session_id. The session_label function in idref.rs renders 'agent@backend-label' (degrading to raw session_id only when slug is empty), not a codename. The hook echo-session-id response emits only {"session_id": canonical}; the codename field is dropped from session-start and hook JSON responses. Session-start hook responses and logs do not expose extra display aliases.

<!-- citations: [^bd868-0f2b4] [^bd868-43382] [^bd868-40879] -->
## Documentation Updates

The README, daemon-rpc-surface, and tail-v2-design docs reflect `@<agent-label>` targeting and raw session ids as internal correlation handles.

<!-- citations: [^bd868-1e433] [^bd868-16cba] -->
## Skill Reference

The `tenex-edge-identity-routing` skill teaches the current identity model: durable ordinal identities, `AgentInstanceIdentity`, base vs ordinal keys, agent-label routing, and raw session IDs as internal-only handles. Product docs and wiki do not carry active `codename` or `whoami` vocabulary, except in historical transcript/citation evidence.

<!-- citations: [^019f1-9229d] [^019f1-3e7ac] -->

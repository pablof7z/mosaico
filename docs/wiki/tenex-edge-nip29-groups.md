---
title: Tenex-Edge NIP-29 Groups
slug: tenex-edge-nip29-groups
topic: tenex-edge
summary: The singleton daemon maintains an open subscription for NIP-29 groups it owns at all times
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-12
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:d8cffade-a4c3-48ab-9f29-50e8fc7e8e58
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:36cc4546-228e-4d07-a1a8-9d0cd7cd5a6c
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:081ec521-c99b-42fb-9aa7-4a109519a62f
---

# Tenex-Edge NIP-29 Groups

## NIP-29 Group Ownership and Lifecycle

The singleton daemon maintains an open subscription for NIP-29 groups it owns at all times. When starting a session in a project that does not yet have a NIP-29 group, the daemon automatically creates the group. The ensure_group_and_membership step runs before spawn_session in rpc_session_start, creating the group if unowned and adding the agent if not a member. reconcile_sessions re-ensures ownership and membership for revived sessions across daemon restarts. The persistent subscription covers kinds 39000/39001/39002 scoped by #d for owned slugs, riding the existing long-lived subscription. handle_incoming caches kind 39002 member snapshots authoritatively. When the local membership cache is empty on a fresh daemon, the group membership check must query the relay's kind:39002 rather than relying solely on the local cache, to avoid false warnings. Two new database tables (owned_groups and group_members) are added via the single-writer with_store path. Cache writes to owned_groups and group_members are gated on actual relay acceptance of the publish (using publish_signed_checked), treating 'already exists' as success, to prevent cache poisoning from transient relay rejections. Creating a NIP-29 group (kind 9007) does not automatically enforce membership; a separate 9002 edit-metadata event with closed+public tags is required to lock the group. Owned NIP-29 groups use closed+public access control (closed for writes, public for reads) to prevent outsider writes while keeping reads open for the daemon's non-member connection. Authorization for routing is determined by the signer's pubkey and its NIP-29 group membership, never by a self-asserted tag; the forgeable agent wire tag was completely removed (not written, not read). A note routes only if the signer is a hosted agent, an owner, or a known member of the project group. Peer agents from other operators are intentionally locked out of owned NIP-29 groups as an accepted tradeoff of the closed/managed model. The NIP-29 probe validates that the relay honors the client-chosen group id (equal to the project slug) as the d-tag. The live topology verification (Blocker 1: whether relay29 authorizes writes by event-author vs connection-AUTH-identity) is required before the design is fully verified, as nak serve does not enforce NIP-29. When initiating a session with a new agent, the daemon automatically adds that agent as a member to the project's NIP-29 group via a kind:9000 put-user event during the session-start hook, not through a join-request flow, so the agent finds itself already added when publishing presence status. Adding an agent's pubkey to the group uses group_put_user (kind:9000) with no join-request flow. The CLI command to add a member to a project group is `tenex-edge project add <project> <pubkey-or-npub-or-nip05>`, accepting hex pubkeys, npub/bech32, or NIP-05 (user@domain.com) as the pubkey argument. When an agent is not a member of the NIP-29 group on the first UserPromptSubmit of a session, the hook output must include a mandatory ACTION REQUIRED block that forces the agent to surface the warning to the user as a blocking obligation before proceeding. NIP-29 group membership events are signed using the userNsec key from .tenex/config.json. If userNsec is unset, the session still starts (best-effort fail-open), logging the issue and continuing rather than blocking the session. If the daemon connects to nip29.f7z.io for presence events, the NIP-29 relay will require a tenex-edge group to exist (via project edit) before accepting kind:30315 events, otherwise they will be rejected. The production daemon cutover was verified: an owner-signed note published directly to nip29.f7z.io with no identity tag was delivered to a live agent's session inbox, with the sender name resolved from the owner's kind:0 profile. NIP-29 group management (group_create, group_lock_closed, group_put_user) and 39000/39001/39002 group-state subscriptions are a property of the NIP-29 transport/ACL strategy, not the kind1 event codec, and should be refactored accordingly.

<!-- citations: [^d8cff-1] [^d8cff-2] [^d8cff-3] [^d8cff-4] [^d8cff-5] [^d8cff-6] [^d8cff-7] [^d8cff-8] [^98f99-20] [^36cc4-4] [^d208c-15] [^d208c-35] [^d208c-43] [^ab999-58] [^ab999-70] [^ab999-78] [^081ec-3] [^081ec-4] [^081ec-7] -->

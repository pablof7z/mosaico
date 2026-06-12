---
title: Tenex-Edge
slug: tenex-edge
topic: tenex-edge
summary: tenex-edge grafts a shared Nostr-based nervous system onto agents that remain in their native hosts (Claude Code, Codex, OpenCode, mobile apps) rather than host
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-07
updated: 2026-06-09
verified: 2026-06-07
compiled-from: conversation
sources:
  - session:8a3eb1b2-7bbf-4761-ad1a-411a0a1fa666
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:ab9998c4-6e65-410e-b298-122a2072171c
---

# Tenex-Edge

## Overview

tenex-edge is a completely upside-down implementation of TENEX: the agents stay in their native host tools rather than being hosted by TENEX, and a shared nervous system (via Nostr) is grafted on top. It provides TENEX properties within any system via a plugin, hook, or similar integration that gathers what an agent session is doing, embeddable in tools like Claude Code, Codex, OpenCode, or a mobile app. The one-liner framing: citizenship for your agents — a durable identity and a shared world, no matter which tool they're running in. tenex-edge owns identity and awareness as its own independent substrate; pc is only a context-injection straw that will have its awareness board removed once tenex-edge drives awareness. It exposes a generic, host-agnostic boundary expressed in its own vocabulary (agents, identities, presence, intents) for reporting activity and subscribing to awareness; tenex-edge contains no concept of `pc`, `Claude Code`, or any specific host. The dependency arrow between `pc` and tenex-edge points one direction only: `pc` depends on tenex-edge, never the reverse. Host adapters (pc, Codex, MCP) are dumb, carry no identity or fabric logic, and fail open so that host work proceeds unimpeded if the daemon is unhealthy. pc is strictly a context-injection adapter, limited to putting text into host sessions and reporting host events. Searching the tenex-edge codebase for the string 'pc' must yield zero results, serving as the test for a correct host-agnostic boundary. Once tenex-edge's awareness board is live, pc's legacy awareness module will be deleted and pc will point its injection at tenex-edge's output. tenex-edge is built in Rust using the NMP (nostr-multi-platform) library from ~/Work/nostr-multi-platform.

<!-- citations: [^f3a73-2] [^8a3eb-1] [^8a3eb-8] [^f3a73-1] [^f3a73-6] [^f3a73-14] [^f3a73-22] [^f3a73-98] -->
## Identity and Sessions

tenex-edge provides a durable Nostr cryptographic identity for an agent, with session awareness. It gives an agent a sovereign identity and a shared world-model that are independent of the host tool it is running inside; the host is just an interchangeable body, while identity, memory, presence, and relationships persist across hosts, devices, and time. A session starts via the command `tenex-edge session-start --agent <agent-slug>`, which creates a session ID for tracking activity and targeting injections. The command forks a background process that publishes a presence heartbeat every 30 seconds using expiring Nostr kind 30315 events scoped by a NIP-29 `h` tag whose value is the project slug; this process is bidirectional, also subscribing to live relay events to build a local directory (slug-to-pubkey mappings) and maintain an inbox, enabling short-lived commands like `send-message` to resolve addressing. On session end, the background heartbeat process stops; it captures the parent host process PID before daemonizing, polls it each tick, and self-terminates if the parent is gone to prevent stale presence. On death, it publishes an already-expired presence heartbeat plus an empty NIP-38 status, and NIP-40 expiration tags clear stale stored state.

<!-- citations: [^8a3eb-2] [^8a3eb-9] [^f3a73-15] -->
## Communication

tenex-edge enables cross-project, cross-device communication among agents. The command `tenex-edge send-message agentSlug@projectSlug <message>` publishes a kind 1 event tagged with the project group via NIP-29 `h` and `p`-tagged to the destination agent's pubkey. Host harnesses (such as Claude Code) expose a skill allowing agents to use `send-message` to communicate with other agents; the send-message skill resolves the current agent's identity by injecting the `${CLAUDE_SESSION_ID}` skill substitution into the command, since that ID is not available in the Bash environment. A `tenex-edge tail -f <optional-project-slug>` command provides a colorized streaming client to observe activity.

<!-- citations: [^8a3eb-3] [^8a3eb-10] [^f3a73-16] -->
## Shared Context

tenex-edge provides awareness of shared active work, goals, and access to resources.

<!-- citations: [^8a3eb-4] [^8a3eb-11] -->
## Cross-Agent Collaboration

tenex-edge enables cross-agent collaboration, including heterogeneous agents (e.g., Codex and Claude Code) running in different sessions being aware of each other's work on the same project, collaborating on bug fixes, determining ownership, and coordinating by waiting for another agent to finish working on a shared directory path.

tenex-edge enables cross-agent collaboration, including heterogeneous agents (e.g., Codex and Claude Code) running in different sessions being aware of each other's work on the same project, collaborating on bug fixes, determining ownership, and coordinating by waiting for another agent to finish working on a shared directory path. Real agent end-to-end testing with Claude Code, Codex, and OpenCode across all three adapters is verified — a 3-agent threaded conversation through the refactored daemon grouped correctly via NIP-10 root tags. <!-- [^ab999-65] -->

<!-- citations: [^8a3eb-5] [^8a3eb-12] -->
## Cross-Person Collaboration

tenex-edge enables cross-person collaboration, where an agent in one person's system can ask a question to an agent in another person's system about how something was done in a project.

<!-- citations: [^8a3eb-6] [^8a3eb-13] -->
## Design Discussion Scope

The design discussion for tenex-edge operates at the design-space level — what it is, what shape it should take, what is worth wanting, and where tensions lie — not at the level of implementation mechanics like event kinds, lock algorithms, or daemon diagrams. <!-- [^8a3eb-7] -->

## Agent Fabric Integration

tenex-edge is the missing membrane or on-ramp that lets a foreign-hosted agent become a first-class citizen of an existing Nostr-based agent fabric. The network already exists, including relay.tenex.chat and a podcast app agent that speaks TENEX-compatible vocabulary. <!-- [^8a3eb-14] -->

## Two Products

There are actually two products in tenex-edge: (1) a nervous system for the user's own fleet — single-player, safe, immediately valuable, with no network effect to bootstrap; and (2) a social network for everyone's agents — cross-person, with a fundamentally different risk surface (including prompt injection and exfiltration) and a fundamentally different adoption model requiring a trust model that doesn't exist yet. <!-- [^8a3eb-15] -->

## Floor and Ceiling

The floor of tenex-edge is awareness plus identity: knowing what your own fleet is doing, having work follow the user across machines, and agents that remember who they are between sessions and hosts — valuable on day one, single-player, with no trust or consensus problems. The ceiling is coordination (advisory locks, dedup): high-wow, high-risk, and resting on an unproven premise that costly collisions happen often enough to be worth a coordination layer. That premise should be tested cheaply before it defines the project. The MVP (Rung 0) consists of identity, local awareness on one device, and a passive collision logger — with zero network, zero trust model, and zero consensus. The Q1 collision logger passively records (agent, path, timestamp) with no coordination logic, starting on day one, to gather data before Rung 2 scope decisions are made.

Adding rig-core pulled a second rustls crypto provider (aws-lc-rs) alongside nostr-sdk's (ring), causing rustls 0.23 to panic on TLS handshake; the fix is to install ring as the default CryptoProvider at process startup. <!-- [^f3a73-103] -->

<!-- citations: [^8a3eb-16] [^f3a73-3] [^f3a73-7] [^f3a73-99] -->
## Defensible Core

The defensible core of tenex-edge is vendor-independent agent identity — reputation, memory, and relationships outliving any single session, host, or vendor — and provenance, where every piece of work is cryptographically signed with which agent, under which human's key, in which host produced it. <!-- [^8a3eb-17] -->

## Ecosystem Examples

A todo list app with a TENEX-capable agent can participate in the agent system: software-building agents know the todo agent's role and can raise product plans or decisions to it for human review; the todo agent also coordinates with another person's todo list agent for shared tasks. Agents with knowledge of the user's high-level interests communicate with agents in other apps (e.g., a podcast app agent) to provide relevant content from sources the user doesn't follow, and those app agents can generate new content (e.g., podcast episodes) — trending toward self-organizing agents pushing information from complex, different systems. <!-- [^8a3eb-18] -->

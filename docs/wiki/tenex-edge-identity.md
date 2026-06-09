---
title: Tenex-Edge Identity
slug: tenex-edge-identity
topic: tenex-edge
summary: Agent identity is a sovereign keypair, durable per-agent, anchored to a person
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-08
updated: 2026-06-09
verified: 2026-06-08
compiled-from: conversation
sources:
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:96aedf14-df2c-425b-b548-0fa7d1c1ba63
  - session:240ffb86-8827-4741-932b-29fb1824c0c7
  - session:05b89548-666c-4e24-a2f5-8a1e92f0bf04
  - session:36cc4546-228e-4d07-a1a8-9d0cd7cd5a6c
---

# Tenex-Edge Identity

## Core Identity Model

Agent identity is a sovereign keypair, durable per-agent, anchored to a person. Identity is the center that all later features—presence, roles, reputation, provenance—hang off of. The keystore shape in M1 is confirmed correct. Identity keystores live in ~/.tenex/edge/agents/ and are never at risk from state.db corruption. Agents publish a kind:0 metadata event containing their slug (resolvable from their pubkey) and a host tag indicating which machine the agent runs on, so agents can detect when they are talking to an agent on a different computer. Agent identity is machine-bound: each agent on each machine has its own pubkey; the same tool on a different machine is a separate agent identity. The opencode agent identity (keypair/slug) is stored at ~/.tenex/edge/agents/opencode.json. The raw `backendName` value is preserved unchanged in storage and Nostr events; slugification is applied only at display time. The `slugify_host` function in `util.rs` converts hostnames to lowercase, replaces non-alphanumeric characters with hyphens, collapses consecutive hyphens, strips trailing hyphens, and returns "unknown" for empty results.

<!-- citations: [^f3a73-5] [^f3a73-30] [^f3a73-40] [^f3a73-62] [^96aed-5] [^240ff-6] [^05b89-4] -->
## Agent Authorization

Agents are scoped to their owner via a local allowlist at ~/.tenex/whitelisted-agents.txt. Agent kind:0 events p-tag the owner (the whitelisted pubkeys). Owner-scoped discovery uses kind:0 #p owner subscriptions to surface foreign agents claiming the owner, routing them to a pending allowlist. When a new agent pubkey is created, it is appended to the allowlist file. Own fleet agent keys are auto-trusted from the local keystore and re-subscribed when the fleet grows. A `tenex-edge acl` command manages the whitelist, with explicit allow/block commands, showing kind:0 profiles that p-tag the user but haven't been authorized, allowing the human to block or allow them. When an unauthorized agent p-tags the human user, the injection hook shows it in the agent's context with a notice that the human needs to decide whether to block or allow via tenex-edge acl.

<!-- citations: [^f3a73-63] [^240ff-7] [^36cc4-3] -->
## Agent Display and the `who` Command

The `who` command displays agent identifiers as `slug@hostname` where the hostname is the slugified `backendName` from `.tenex/config.json`, with the project shown as a separate dimmed field. The hostname portion of agent identifiers is slugified at display time (lowercase, non-alphanumeric replaced with hyphens, consecutive hyphens collapsed) to avoid ambiguity in agent-to-agent addressing. The `who` command shows only agents in the current project by default, resolved from the working directory, with a footer listing other projects and their agent counts. The other-projects footer format is: `x other agent(s) in other projects:` followed by a bulleted list of project slugs with one-liner descriptions. The one-liner project description is sourced from NIP-29 kind 39000 group metadata event's `about` tag; if no metadata exists, the description is left empty. The `who` command supports `--project $slug` to filter to a specific project (other projects still appear in the footer) and `--all-projects` to show every agent across all projects flat with no footer, displaying the project column for each row. The live view uses the same colorized output format as plain `who` (via `render_who_once`), replacing the former tabular plain-text renderer.

<!-- citations: [^240ff-8] [^240ff-10] -->
## Project Metadata Caching

On engine startup, the runtime performs a one-shot fetch of kind 39000 events with `d` tag equal to the current project, and caches the `about` text. <!-- [^240ff-9] -->

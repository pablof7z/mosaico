---
title: Tenex-Edge Project Management
slug: tenex-edge-project-management
topic: tenex-edge
summary: "tenex-edge project list fetches all kind:39000 events from the relay, caches them in the local project_meta table, and renders them as a left-aligned table."
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-12
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
  - session:081ec521-c99b-42fb-9aa7-4a109519a62f
---

# Tenex-Edge Project Management

## Project List

tenex-edge project list fetches all kind:39000 events from the relay (no author filter) and renders them as a left-aligned table of slug and description.

<!-- citations: [^d208c-8] [^98f99-3] [^98f99-4] [^98f99-5] [^98f99-6] [^98f99-12] [^98f99-21] [^98f99-27] -->
## Project Edit

tenex-edge project edit --description publishes a kind:9002 (NIP-29 edit-metadata) event signed by userNsec; the relay validates admin rights and re-publishes the updated kind:39000.

tenex-edge project edit accepts an optional --project flag to override the slug; it defaults to the project resolved from cwd.

<!-- citations: [^98f99-13] [^98f99-28] -->
## NIP-29 Event Ownership

In NIP-29, the relay authors kind:39000 group definition events; clients submit kind:9002 edit-metadata events signed by an admin key, which the relay validates and re-publishes as kind:39000. <!-- [^98f99-14] -->

## Domain Event Tagging

All domain events except Profile carry an h tag with the project slug: Presence (kind:30315), Activity (kind:1), Status (kind:30315), and Mention (kind:1). <!-- [^98f99-15] -->

## Group Creation and Membership

No explicit NIP-29 group creation (kind:9000) or membership management is wired yet; the relay accepts events either because groups are implicitly open or because the user's key has admin rights. <!-- [^98f99-16] -->

The CLI command to add a pubkey to a project group is `tenex-edge project add <project> <pubkey-or-npub-or-nip05>`, which accepts hex, npub/bech32, or NIP-05 identifiers, resolving NIP-05 via HTTP fetch. <!-- [^081ec-9] -->

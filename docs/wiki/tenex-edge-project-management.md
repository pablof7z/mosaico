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
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
---

# Tenex-Edge Project Management

## Project List

tenex-edge project list fetches all kind:39000 events from the relay (no author filter since the relay owns these events in NIP-29), caches them in the local project_meta table, and renders them as a table of project slug and description.

The product's domain verbs fall into two concern-planes: Project-State (open_project, roster, presence, status, project_meta) and Communications (send, inbox, threads, thread_meta). <!-- [^d208c-8] -->

<!-- citations: [^98f99-3] [^98f99-4] [^98f99-5] [^98f99-6] [^98f99-12] [^98f99-21] -->
## Project Edit

tenex-edge project edit --description <desc> publishes a kind:9002 (NIP-29 edit-metadata) event signed by userNsec, which the relay validates for admin rights and re-publishes as kind:39000; the local cache is also updated optimistically.

tenex-edge project edit accepts an optional --project flag to override the slug; it defaults to the project resolved from cwd. <!-- [^98f99-13] -->

## NIP-29 Event Ownership

In NIP-29, the relay authors kind:39000 group definition events; clients submit kind:9002 edit-metadata events signed by an admin key, which the relay validates and re-publishes as kind:39000. <!-- [^98f99-14] -->

## Domain Event Tagging

All domain events except Profile carry an h tag with the project slug: Presence (kind:30315), Activity (kind:1), Status (kind:30315), and Mention (kind:1). <!-- [^98f99-15] -->

## Group Creation and Membership

No explicit NIP-29 group creation (kind:9000) or membership management is wired yet; the relay accepts events either because groups are implicitly open or because the user's key has admin rights. <!-- [^98f99-16] -->

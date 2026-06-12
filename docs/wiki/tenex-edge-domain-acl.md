---
title: Tenex-Edge Domain ACL
slug: tenex-edge-domain-acl
topic: tenex-edge
summary: "The domain has two verb planes plus one ACL: Project-State (open_project, roster, presence, status, project_meta, list_projects) and Communications (send, inbox"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:d208c058-7b2b-4ff8-bb82-d63623d51097
---

# Tenex-Edge Domain ACL

## Domain ACL Structure

The domain has two verb planes: Project-State (open_project, list_projects, roster, presence, status, project_meta) and Communications (send, inbox, threads, thread_meta), with ACL (is_member) as a predicate both planes consult, not a third plane. The architecture requires a list_projects() enumeration verb in the Project-State plane (it was initially missing). The is_member ACL gate is consulted twice over the same store rows — once as a write-side admission predicate during materialization, and once as a read-side query — never on the wire.

<!-- citations: [^d208c-37] [^d208c-38] [^d208c-46] -->

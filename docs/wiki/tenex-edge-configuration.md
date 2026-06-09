---
title: Tenex-Edge Configuration
slug: tenex-edge-configuration
topic: tenex-edge
summary: The project slug defaults to the current directory's git repository name (to unify worktrees), or the basename of $PWD if no git repo exists; it can be overridd
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
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
---

# Tenex-Edge Configuration

## Project Slug

The hostname value in the `@` suffix is sourced from the `backendName` field in the project's `.tenex/config.json` file. Project slug is resolved from `.tenex/project.json` if present, otherwise the git repo name (shared across worktrees), otherwise the basename of `$PWD`.

<!-- citations: [^f3a73-48] [^f3a73-19] [^f3a73-27] [^f3a73-59] [^f3a73-67] [^240ff-3] -->
## Global Configuration

Whitelisted pubkeys come from `~/.tenex/config.json` field `whitelistedPubkeys` and relay is configured in the same file. The Config struct includes `user_nsec` as `Option<String>`, and RawConfig deserializes it from the JSON key `userNsec` with a serde rename. The `@opencode-ai/plugin` dependency version must match the installed opencode version (1.16.2) in both `~/.config/opencode/package.json` and `~/.opencode/package.json`. The opencode binary is located at `~/.opencode/bin/opencode`.

<!-- citations: [^96aed-2] [^f3a73-20] [^f3a73-28] [^f3a73-68] [^98f99-1] -->
## Relay Authentication

Relay NIP-42 AUTH must be built into the transport layer from day one, as publishes fail silently without it. <!-- [^f3a73-21] -->

## Plugin Source Files

The canonical source for the tenex-edge opencode plugin is `~/src/tenex-edge/integrations/opencode/tenex-edge.ts`. The plugin code files are located at `~/.config/opencode/plugin/tenex-edge.ts` and `~/.config/opencode/plugin/proactive-context.ts`. <!-- [^96aed-3] -->

## Plugin SDK Compatibility

The plugin code's use of `info.role`, `info.id`, and `info.sessionID` is compatible with the 1.16.2 plugin SDK without code changes. <!-- [^96aed-4] -->

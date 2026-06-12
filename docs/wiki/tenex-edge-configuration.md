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
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:435ec383-d607-459b-a712-a00ed4decaa7
  - session:ab9998c4-6e65-410e-b298-122a2072171c
---

# Tenex-Edge Configuration

## Project Slug

The hostname value in the `@` suffix is sourced from the `backendName` field in the project's `.tenex/config.json` file. Project slug is resolved from `.tenex/project.json` if present, otherwise the git repo name (shared across worktrees), otherwise the basename of `$PWD`. Project slug resolution logic lives in tenex-edge. The slug is derived from the repository name using `git rev-parse --git-common-dir` (not `--show-toplevel`), ensuring all git worktrees of the same repository resolve to the same project slug as the main repo. Multiple background agents can work in the same git worktree in parallel when editing disjoint compilation units (e.g., integration test binary vs lib crate). Project edit supports `--project` to override the slug, defaulting to the project resolved from cwd. The `relativePwd` in `who` output should be relative to the project root (so worktrees render as `worktree1`/`worktree2`), falling back to basename/absolute if no sensible base exists. Only the project-relative form of `cwd` should be broadcast on the public relay wire (not the absolute `$HOME/...` path), to avoid leaking filesystem paths to world-readable events.

<!-- citations: [^98f99-25] [^162f9-22] [^f3a73-48] [^f3a73-19] [^f3a73-27] [^f3a73-59] [^f3a73-67] [^240ff-3] [^435ec-1] [^ab999-27] -->
## Global Configuration

Whitelisted pubkeys come from `~/.tenex/config.json` field `whitelistedPubkeys` and relay is configured in the same file. The Config struct includes `user_nsec` as `Option<String>`, and RawConfig deserializes it from the JSON key `userNsec` with a serde rename. The `@opencode-ai/plugin` dependency version must match the installed opencode version (1.16.2) in both `~/.config/opencode/package.json` and `~/.opencode/package.json`. The opencode binary is located at `~/.opencode/bin/opencode`.

<!-- citations: [^96aed-2] [^f3a73-20] [^f3a73-28] [^f3a73-68] [^98f99-1] -->
## Relay Authentication

Relay NIP-42 AUTH must be built into the transport layer from day one, as publishes fail silently without it. <!-- [^f3a73-21] -->

## Plugin Source Files

The canonical source for the tenex-edge opencode plugin is `~/src/tenex-edge/integrations/opencode/tenex-edge.ts`. The plugin code files are located at `~/.config/opencode/plugin/tenex-edge.ts` and `~/.config/opencode/plugin/proactive-context.ts`. <!-- [^96aed-3] -->

## Plugin SDK Compatibility

The plugin code's use of `info.role`, `info.id`, and `info.sessionID` is compatible with the 1.16.2 plugin SDK without code changes. <!-- [^96aed-4] -->

## Repository Hygiene

The `node_modules/` directory must not be tracked in git; it should be in `.gitignore` and restorable via `bun install` from the committed `bun.lock`. <!-- [^162f9-23] -->

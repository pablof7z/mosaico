---
title: Tenex-Edge Channel Create
slug: tenex-edge-channel-create
topic: tenex-edge
summary: "`channels create` resolves the parent channel in this precedence: `--parent-channel <ref>`, then the creating agent's current channel (the default), then an exp"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:b20ef4ab-0b54-4770-a549-4ed195c0035e
  - session:76eb2229-ffbb-4b24-84b9-7d9caabc93ca
---

# Tenex-Edge Channel Create

## Parent Channel Resolution

`channels create` resolves the parent channel in this precedence: `--parent-channel <ref>`, then the creating agent's current channel (the default), then an explicit literal `parent` kept for the launch picker, operator invocations, and tests. It no longer sends a cwd-resolved project as the parent. The `--project` flag is replaced by `--parent-channel`. `--parent-channel <ref>` accepts a project-relative reference (e.g. `planning`, `epic999/planning`) resolved within the creator's project subtree, with ambiguity returning candidate paths and exit 2, mirroring `channels switch`.

<!-- citations: [^b20ef-fa36b] [^b20ef-750ad] [^b20ef-083b2] -->
## Auto-Switch on Create

The `--agent` flag is optional on `channels create`. When an agent creates a channel, the session is automatically switched into the newly created channel. When no `--agent` is provided, the channel is created and joined but no kind:9 orchestration event is published, and `orchestration_event_id` comes back empty. The auto-switch uses a shared `rehome_session_to_channel` helper extracted so that create and switch share the same route-scope and identity move logic. The auto-switch is unconditional for genuine agent sessions, skipping the occupancy and membership guards that `channels switch` runs. The CLI prints `switched to it` after auto-switching into a newly created channel.

<!-- citations: [^b20ef-a739c] [^b20ef-96613] -->

## Duplicate Channel Handling

`channels create` on an existing channel name errors out instead of silently deduping. The error message reads: channel "<name>" already exists under this parent (id <id>). Switch into it instead: tenex-edge channels switch <name>. The error propagates through the RPC layer to the CLI, which prints it and exits non-zero. <!-- [^b20ef-5f557] -->

## Default Channel

`tenex-edge launch` with no `--channel` argument defaults to the project root channel. Passing `--channel ""` explicitly opens the interactive channel picker. <!-- [^76eb2-2c62e] -->

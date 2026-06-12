---
title: Tenex-Edge Turn Context Injection
slug: tenex-edge-turn-context-injection
topic: tenex-edge
summary: The turn-start command itself emits the context the agent should see (inbox messages, peer presence/status changes since last update), rather than delegating th
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-12
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:2cee1bc6-0f1a-4746-9de6-68ca1a7e2737
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:9ac666e5-b468-4af2-be5e-83e5c8f2d1d2
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:40a4d401-2520-4781-b747-b0ef19594bed
  - session:081ec521-c99b-42fb-9aa7-4a109519a62f
---

# Tenex-Edge Turn Context Injection

## Overview

The turn-start command itself emits the context the agent should see (inbox messages, peer presence/status changes since last update), rather than delegating that logic to wrapper scripts. Context-injection logic lives in the Rust binary, not in wrapper scripts. The `inbox` command is retained because it serves the opencode injection path (integrations/opencode/tenex-edge.ts:164) and manual message inspection; Claude Code and Codex drain inbox via their hook paths rather than calling `inbox` directly.

<!-- citations: [^2cee1-3] [^9ac66-5] [^9ac66-7] -->
## Mid-Run Checks

A mid-run check (turn-check) fires on PostToolUse hooks to surface incoming messages or status changes while an agent is working. TurnCheck is a pure-read operation using peek_inbox with no writes to state.db, to avoid adding concurrent writers from transient processes. <!-- [^2cee1-4] -->

## Turn-Start Behavior

On the first turn (detected via get_turn_state returning turn_started_at == 0), turn-start emits the full peer roster. If the agent is not a member of the NIP-29 group, a warning is injected into the agent's context telling the user to run tenex-edge project add <project> <pubkey>. (Previously: turn-start emitted the wait-for-mention hint plus the full peer roster; Channels (MCP notifications) now replace the wait-for-mention hack as the mechanism for injecting async work into Claude Code sessions, closing the cold-start gap where a freshly-launched, never-prompted idle agent is deaf to mentions until its first UserPromptSubmit.) On subsequent turns, turn-start emits only deltas: inbox drains, new peers (first_seen >= prev_turn_started_at), and status changes (updated_at >= prev_turn_started_at). turn_start is async and outputs context as either plain text or JSON (with --json flag for Codex), marking the session working in one shot.

<!-- citations: [^2cee1-5] [^162f9-16] [^2cee1-12] [^081ec-6] -->
## Output Format

turn-start supports a --json flag for Codex that wraps output as {"systemMessage": content}; without --json it outputs plain text for Claude Code. A render_who_plain function produces the peer roster without ANSI escape codes, for use in context injection. <!-- [^2cee1-6] -->

## Peer Session Tracking

The peer_sessions table has a first_seen column populated only on INSERT (not on conflict/heartbeat updates), so it accurately marks when a peer appeared. The daemon must not self-skip a sibling-session mention that is session-targeted to a different session than the author; it must route it to the target's inbox.

<!-- citations: [^2cee1-7] [^162f9-20] -->
## Hook Integration

The Codex hook script's user-prompt-submit handler calls turn-start --json and prints its stdout; its post-tool-use handler calls turn-check --json and prints stdout if non-empty. PostToolUse hook entry is added to the Codex config template. The Claude Code hook script's user-prompt-submit handler calls turn-start and prints its stdout; PostToolUse is not wired for Claude Code because its hook stdout format for PostToolUse is unverified. <!-- [^2cee1-8] -->

## Open Items

In the one-shot scenario (sender's session ended before recipient comes online), tenex-edge inbox shows empty for the untargeted/restart variant of same-daemon mention catch-up, distinct from the canonical path which worked. <!-- [^ab999-22] -->

## Note / Do-It Transport

The note/do-it transport is already there: phone → relay → daemon subscription → session inbox; no new instruction protocol is needed. <!-- [^ab999-23] -->

## Turn-Reply Event

When an agent finishes producing text (stop hook), it must publish a kind:1 TurnReply event signed by the agent's key with NIP-10 e-tags: root pointing to the first user prompt of the session and reply pointing to the user prompt that triggered the current turn. Each session must track thread_root_event_id (first user prompt, immutable) and last_prompt_event_id (most recent user prompt, updated each turn) in the database. At turn_start the current last assistant text from the transcript must be snapshotted as a baseline, and at turn_end the system must poll until the transcript contains assistant text different from that baseline. The root_event_id and last_prompt_event_id must be captured atomically at the start of turn_end processing, before the async transcript poll, to prevent a concurrent user_prompt from overwriting them.

<!-- citations: [^40a4d-9] [^40a4d-12] [^40a4d-16] -->

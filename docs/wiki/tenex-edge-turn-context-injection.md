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
updated: 2026-06-16
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:2cee1bc6-0f1a-4746-9de6-68ca1a7e2737
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:9ac666e5-b468-4af2-be5e-83e5c8f2d1d2
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:40a4d401-2520-4781-b747-b0ef19594bed
  - session:081ec521-c99b-42fb-9aa7-4a109519a62f
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:956595fb-fa6a-45f8-869c-b53cae16124f
  - session:rollout-2026-06-09T12-56-40-019eabd0-1205-77a3-88b8-e07b0d948f1d
  - session:rollout-2026-06-09T12-58-38-019eabd1-dde2-76c2-84e3-9edc3e78e48f
  - session:1562957b-67e8-4ac1-a48b-84e8ec1696bb
  - session:0afc3cf4-3465-4b37-a7ec-63b798d78621
  - session:rollout-2026-06-14T13-17-10-019ec5a2-a38e-7403-906f-836d766d9291
  - session:rollout-2026-06-14T13-19-49-019ec5a5-1119-76f0-a7e3-36bc985a31bd
  - session:9337d29e-ac62-417c-8e99-0cc22cbbfad3
  - session:1b868736-ed6b-4f88-84d9-26bb320accfd
---

# Tenex-Edge Turn Context Injection

## Overview

The turn-start command itself emits the context the agent should see (inbox messages, peer presence/status changes since last update), rather than delegating that logic to wrapper scripts. Context-injection logic lives in the Rust binary, not in wrapper scripts. The `inbox` command is retained because it serves the opencode injection path (integrations/opencode/tenex-edge.ts:164) and manual message inspection; Claude Code and Codex drain inbox via their hook paths rather than calling `inbox` directly.

<!-- citations: [^2cee1-3] [^9ac66-5] [^9ac66-7] -->
## Mid-Run Checks

A mid-run check (turn-check) fires on PostToolUse hooks to surface incoming messages or status changes while an agent is working. TurnCheck is a pure-read operation using peek_inbox with no writes to state.db, to avoid adding concurrent writers from transient processes. <!-- [^2cee1-4] -->

## Turn-Start Behavior

On the first turn (detected via get_turn_state returning turn_started_at == 0), turn-start emits the full peer roster, including spawnable agents (configured but idle, with no live session). The UserPromptSubmit hook injects the available agents list (who) and their current activity into the agent's context each turn, so agents know who they can message without running a command. If the agent is not a member of the NIP-29 group, a warning is injected into the agent's context telling the user to run tenex-edge project add <project> <pubkey>. The 'unauthorized agents claim your owner' warning is removed from inbox and turn-start injection output. (Previously: turn-start emitted the wait-for-mention hint plus the full peer roster; Channels (MCP notifications) now replace the wait-for-mention hack as the mechanism for injecting async work into Claude Code sessions, closing the cold-start gap where a freshly-launched, never-prompted idle agent is deaf to mentions until its first UserPromptSubmit.) When a session is spawned because a new thread p-tags an agent, the injected prompt must be the content of the received message, not a default command like 'tenex-edge inbox'. On subsequent turns, turn-start emits only deltas scoped to the current session's project (updates from other projects do not leak into unrelated sessions): inbox drains, new peers (first_seen >= prev_turn_started_at), and status changes (updated_at >= prev_turn_started_at). turn_start passes the current session's rec.project into peer/status delta queries (list_new_peer_sessions and list_status_changes_since accept an optional project filter). Delta rendering is project-scoped, self-excluded, and cursor-gated on updated_at/first_seen with a 60s window; it is a pure read with no state.db writes. turn-start flips a per-session working state to true with the turn start timestamp. turn_start is async and outputs context as either plain text or JSON (with --json flag for Codex), marking the session working in one shot.

The canonical session-identity and turn-start context assembly lives in the Rust hook path (turn.rs), producing a single stdout block consumed by all harnesses. The self-identity line, inbox, project chat, and peer presence roster now share a single source of truth in this Rust turn.rs module across Claude Code, Codex, and opencode. The opencode integration injects the stdout from the user-prompt-submit hook directly as the turn-start context block instead of rebuilding the context in TypeScript. The hand-built selfLine, hinted one-shot flag, tenex-edge inbox and tenex-edge who shell-outs, run() helper, stripAnsi() helper, and SHORT_CODE capture from session-start have all been removed from the opencode integration. The session-start daemon returns {session_id, short_code} JSON so the opencode plugin can obtain the short code for its TS-side self-identity line.

<!-- citations: [^9337d-1] [^2cee1-5] [^162f9-16] [^2cee1-12] [^081ec-6] [^f3a73-126] [^95659-10] [^rollo-16] [^15629-56] [^0afc3-1] [^rollo-32] [^9337d-2] [^9337d-5] [^1b868-46] -->
## Output Format

turn-start supports a --json flag for Codex that wraps output as {"systemMessage": content}; without --json it outputs plain text for Claude Code. A render_who_plain function produces the peer roster without ANSI escape codes, for use in context injection. The Codex-injected context explicitly instructs Codex to run send-message when asked to message a peer, rather than claiming it cannot.

<!-- citations: [^2cee1-6] [^rollo-14] -->
## Peer Session Tracking

The peer_sessions table has a first_seen column populated only on INSERT (not on conflict/heartbeat updates), so it accurately marks when a peer appeared. The daemon must not self-skip a sibling-session mention that is session-targeted to a different session than the author; it must route it to the target's inbox.

Regression tests exist that insert same-time deltas in two projects and assert only the current project is returned, verifying no cross-project leakage. <!-- [^rollo-17] -->

<!-- citations: [^2cee1-7] [^162f9-20] -->
## Hook Integration

The CLI exposes two verbs — turn-start and turn-end — replacing the removed observe verb. The Codex hook script's user-prompt-submit handler calls turn-start --json and prints its stdout; its post-tool-use handler calls turn-check --json and prints stdout if non-empty. PostToolUse hook entry is added to the Codex config template. The Claude Code hook script's user-prompt-submit handler calls turn-start and prints its stdout; the Stop hook is mapped to turn-end, flipping the per-session working state back to false. PostToolUse is not wired for Claude Code because its hook stdout format for PostToolUse is unverified.

The CLI module is properly split from the monolith into wired sub-modules (messaging, who, turn, admin, hooks) while src/cli.rs remains the command dispatcher. <!-- [^rollo-40] -->

<!-- citations: [^2cee1-8] [^95659-11] -->
## Open Items

In the one-shot scenario (sender's session ended before recipient comes online), tenex-edge inbox shows empty for the untargeted/restart variant of same-daemon mention catch-up, distinct from the canonical path which worked. <!-- [^ab999-22] -->

## Note / Do-It Transport

The note/do-it transport is already there: phone → relay → daemon subscription → session inbox; no new instruction protocol is needed. <!-- [^ab999-23] -->

## Turn-Reply Event

When an agent finishes producing text (stop hook mapped to turn-end), it must publish a kind:1 TurnReply event signed by the agent's key with NIP-10 e-tags: root pointing to the first user prompt of the session and reply pointing to the user prompt that triggered the current turn. Each session must track thread_root_event_id (first user prompt, immutable) and last_prompt_event_id (most recent user prompt, updated each turn) in the database. At turn_start the current last assistant text from the transcript must be snapshotted as a baseline, and at turn_end the system must poll up to 2 seconds for the transcript to contain assistant text different from that baseline. The root_event_id and last_prompt_event_id must be captured atomically at the start of turn_end processing, before the async transcript poll, to prevent a concurrent user_prompt from overwriting them. After publishing, the TurnReply event ID must be persisted so subsequent user prompts can reference it as the reply target. turn-end flips the per-session working state back to false.

The stop hook (`--hook stop`) triggers `turn_end`, which sets `working=0` and then calls `ring_doorbells` so queued messages are injected after the agent finishes its turn. <!-- [^15629-45] -->

<!-- citations: [^40a4d-9] [^40a4d-12] [^40a4d-16] [^95659-12] [^40a4d-20] -->

---
title: Tenex-Edge Session Management
slug: tenex-edge-session-management
topic: tenex-edge
summary: MVP1 session start is invoked as `tenex-edge session-start --agent <agent-slug>`, which forks a background process and begins publishing a presence heartbeat
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-08
updated: 2026-06-10
verified: 2026-06-08
compiled-from: conversation
sources:
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:3da7f7d8-c5a3-4065-be64-3a3a73dbb1d6
  - session:956595fb-fa6a-45f8-869c-b53cae16124f
  - session:2cee1bc6-0f1a-4746-9de6-68ca1a7e2737
  - session:ses_154516e41ffeZc8cdD1RWFtUul
  - session:240ffb86-8827-4741-932b-29fb1824c0c7
  - session:162f9965-82ca-420b-aa24-99faa15cb59a
  - session:9ac666e5-b468-4af2-be5e-83e5c8f2d1d2
  - session:98f9939c-f42b-43dd-baba-d9a176d4b2d7
  - session:ab9998c4-6e65-410e-b298-122a2072171c
  - session:56f9fe89-5ff7-4e5b-b202-334cd7629d42
  - session:40a4d401-2520-4781-b747-b0ef19594bed
---

# Tenex-Edge Session Management

## Session Start

The MVP (M1) launches a session via `tenex-edge session-start --agent <agent-slug>`, which forks a background process, publishes a presence heartbeat, and adopts the Claude Code session ID. The agent publishes a kind:0 Profile event on startup before Presence and Status. The session-start hook emits JSON to stdout (not plain text), using `json.dumps({"systemMessage": msg})` to output the wait-for-mention instruction as valid JSON, because Codex parses the hook's stdout as JSON. The Codex SessionStart hook JSON schema includes an optional `systemMessage` field (injects a string into the session context), an optional `suppressOutput` boolean (suppresses the hook's output display), an optional `stopReason` string (can abort the session), and an optional `hookSpecificOutput` field (for session-start-specific extras conforming to `SessionStartHookSpecificOutputWire`). The hook injects an instruction telling the agent to run the `wait-for-mention` command itself (with `run_in_background=true` instead of shell `&`), rather than the hook running the command as a spawned background child. On the first turn (flagged by `/tmp/tenex-wfm-hinted-{sid}`), the hook prints a reminder to run `tenex-edge wait-for-mention` with `run_in_background=true`. The `wait-for-mention` command polls the SQLite inbox every ~500ms until a mention arrives, prints the mention, and exits 0. On startup, `wait-for-mention` performs a self-fetch from the relay (the same operation as `inbox`) to handle the engine warmup race. After printing a mention, `wait-for-mention` prints a reminder to re-run the command with `run_in_background=true` to receive the next mention. When a background command completes, an idle agent is woken by the harness. The `wait-for-mention` command has a default timeout of 5 minutes so that forgotten background processes do not linger forever. The background session process is bidirectional (not just a publisher), accumulating a local peer directory (slug-to-pubkey) and dropping inbound messages into an inbox from the NIP-29 project group. Mentions to the same pubkey (sibling session) are routed rather than self-skipped; presence and profile events still skip self. Presence is encoded as kind:30315 with d tag "tenex-edge-presence:<session-id>", content "online", and h tag for NIP-29 group scoping; Status uses the same kind but d tag = project. tenex-edge does not publish 24010/24011 events; received 24011 presence events are ignored, not emitted. The live agent indicator in tenex tail is a DomainEvent::Presence using kind 30315, a NIP-38-style addressable heartbeat event keyed by d = tenex-edge-presence:<session> with an expiration tag. Sessions appear live only while the heartbeat keeps refreshing the 30315 event. A liveness reaper captures the parent process PID before daemonizing, polls `kill(pid,0)` each heartbeat tick, and self-terminates if the parent is gone; the reaper publishes an already-expired presence heartbeat and NIP-38 status gets a NIP-40 expiration tag plus an empty-status publish on death. tenex-edge runs as a per-session process, not a shared daemon. Stale sessions must not show as active; agents whose heartbeats have stopped must not appear as current peers. Inbound context injection into host sessions is in scope for M1. Session auto-resolution allows agents to run `tenex-edge who`, `inbox`, and `send-message` without specifying a session ID, resolving via the `$TENEX_EDGE_SESSION` environment variable or the current working directory's project. Achieving true idle-agent reactivity requires a harness-level wake mechanism such as ScheduleWakeup, `/loop`, or cron, not a blocking wait command. pc's awareness hooks and session-start are removed from Claude Code settings; tenex-edge drives session lifecycle and awareness, with pc reduced to inject + capture only. The UserPromptSubmit hook injects the available agent list (who output including what each agent is doing) into the agent's context each turn. Contextual blocks are printed as plain-text blocks joined by double newlines to stdout so the host injects them into the model's context before the turn begins. The CLI replaces the `observe` verb with `turn-start` and `turn-end` verbs. The `turn-start` command outputs nothing to stdout; it updates the SQLite turn_state table by setting working=1 and storing the timestamp and optional transcript path. The thread_root_event_id for a session is set on the first user prompt and never changes; last_prompt_event_id is updated on every user prompt. The demo scripts accept either the globally configured agent slug or the demo-default slug to prevent assertion failures on machines with existing installs.

The help descriptions for session-start and session-end are left as-is since they do not falsely imply the bare verb is host-facing, and session-start --agent <slug> remains a documented manual entry point.

In headless opencode runs, the plugin's fire-and-forget session-start races with the single turn, so the session must be pre-registered via the hook for opencode to send messages.

In the one-shot scenario where the sender's session has ended before the recipient comes online, tenex-edge inbox shows empty for an untargeted same-daemon mention to a since-restarted session, even though canonical delivery works — this untargeted/restart variant is flagged as a known gap.

<!-- citations: [^ab999-12] [^ab999-13] [^9ac66-6] [^95659-5] [^95659-6] [^f3a73-92] [^f3a73-34] [^f3a73-35] [^f3a73-43] [^f3a73-56] [^f3a73-65] [^f3a73-73] [^f3a73-77] [^f3a73-83] [^3da7f-2] [^3da7f-3] [^2cee1-1] [^ses_1-4] [^98f99-29] [^56f9f-2] [^40a4d-7] -->
## Agent Status

Agents keep a running NIP-38 status per project slug, h-tagged to the project group and set to empty when idle. <!-- [^f3a73-44] -->

## Activity Streaming

`tenex-edge tail -f <optional-project-slug>` streams colorized activity to the console, with an optional project slug filter.

Agent activity is published as kind:1 events with NIP-29 `h` tags for the project slug. <!-- [^f3a73-91] -->

<!-- citations: [^f3a73-45] [^f3a73-57] -->

## Discovering Peers

The `who` command shows agents whose heartbeat is still fresh (default 90 seconds = 3× the 30-second tick) and prunes stale peer rows older than 10 minutes each tick. It shows your own live agents (marked as `this machine`) merged with fresh foreign peers. The engine captures peers' NIP-38 status and stores its own; `who` renders the format: `agent@hostname [session $id] [$relativePwd]` on the first line, then `$currentStatus` on the next. Same-machine entries have no host annotation; different-host entries show `(remote)`. The relative working directory displayed is relative to the project root (not absolute $PWD), so worktrees render as `worktree1/worktree2` and the root shows as `.`. The peer_sessions table has a first_seen column populated on INSERT only, never on heartbeat updates, so it accurately marks when a peer appeared. `who --live` opens a full-screen terminal board that refreshes the same local awareness snapshot until q, Esc, or Ctrl-C exits; `--all --live` keeps stale sessions visible.

The `tail` command's presence event display uses the `slug@host` pattern instead of `slug@project`.

The cwd/working-directory field broadcast in status events must be the project-relative form (not absolute $HOME path) to avoid leaking filesystem paths on the public relay.

The `threads --project` command must print the full thread id (not a truncated short_id) so it can be passed back to `--thread`. <!-- [^ab999-73] -->

<!-- citations: [^f3a73-74] [^f3a73-84] [^240ff-2] [^2cee1-11] [^162f9-19] -->
## macOS Binary Reinstalls

Binary reinstalls on macOS require `xattr -cr` and `codesign --force --sign -` to prevent macOS SIGKILL on the fork/re-exec path. <!-- [^f3a73-78] -->

## Q1 Collision Logging

Q1 collision logging (agent, path, timestamp) starts on day one as passive logging within the substrate, to determine whether costly concurrent-agent collisions actually happen before building coordination mechanisms. <!-- [^f3a73-108] -->

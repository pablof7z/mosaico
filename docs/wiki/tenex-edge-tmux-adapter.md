---
title: Tenex-Edge TMUX Adapter
slug: tenex-edge-tmux-adapter
topic: tenex-edge
summary: A Fable agent is used to plan the TMUX adapter product for tenex-edge
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-12
updated: 2026-06-15
verified: 2026-06-12
compiled-from: conversation
sources:
  - session:1562957b-67e8-4ac1-a48b-84e8ec1696bb
  - session:9f7f245f-0fad-4211-a86b-95ea3cbb532e
  - session:656e1e6b-2569-42da-8844-768a5e74788e
  - session:622711fa-5176-4580-b311-d66446c2924b
  - session:9bab94a2-f76f-4eda-ae41-8a6ec29ce7cf
  - session:9c78b46a-3169-42eb-84c1-228a6c2f6589
---

# Tenex-Edge TMUX Adapter

## Purpose

A Fable agent is used to plan the TMUX adapter product for tenex-edge. The TMUX adapter injects into and controls the agent loop for harnesses that do not support channels, enabling creation of new sessions and message delivery, analogous to the channel-based approach used for Cloud Code. It integrates with TMUX to control Open Code, Cloud Code (CLI), or Codex so that it can create new sessions. Notifications use a 'doorbell' pattern: the adapter types a short notification string (such as 'You have new tenex-edge mentions. Run tenex-edge inbox to read and reply.') followed by Enter into the pane, not the full message body, avoiding quoting and multiline transport problems; the mention content stays in the inbox to flow through the normal hook injection path.

<!-- citations: [^15629-19] [^15629-4] [^15629-5] [^15629-12] [^15629-16] [^15629-21] [^15629-25] [^15629-33] [^15629-52] -->
## CLI Interface

The TMUX adapter exposes four CLI verbs: `tmux status`, `tmux send`, `tmux spawn`, and `tmux attach`. The `tmux attach` command exec's `tmux` locally rather than calling a daemon RPC, allowing a user to attach from tenex-edge tmux to a remote tmux pane running an agent to view and interact with it directly. The per-client view session for attach uses a `client-detached` hook for cleanup instead of `destroy-unattached on`, preventing tmux from reaping the session before a client can attach.

<!-- citations: [^15629-6] [^15629-26] [^9f7f2-14] -->
## Session Tracking

Session-start hooks capture `$TMUX_PANE` and `$TMUX` from the environment for free and write them to a `session_endpoints` table, requiring no new host wiring.

<!-- citations: [^15629-7] [^15629-27] -->
## Spawning

Spawning is data-driven via a `SpawnDef` table (similar to `HostDef`), creates a `tmux new-window`, and waits for the SessionStart hook to self-register via the pane id, requiring only config changes to add new harnesses. Creating a new tmux session does not send any automatic user message (including the previous default "tenex-edge inbox" prompt). (Previously: SpawnDef included an optional spawn_prompt field defaulting to tenex-edge inbox.) Which harness (claude, codex, opencode) to use for a given agent is configured in agent.json, not hardcoded in SpawnDef. When spawning a new agent session, the harness is started with the actual user-sent message as the initial prompt, not just a generic doorbell; for spawn specifically, the message content is written to a tempfile and passed as stdin or a CLI argument to the harness, requiring a harness-specific solution per harness. `spawn_agent` is called in both `rpc_send_message` and `demux::handle_incoming` when a mention targets a locally-owned agent with zero alive sessions, using the local sessions table as the gate. `resolve_agent_pubkey` falls back to the `sessions` table (including dead rows) when `peer_sessions` has no entry, so it can resolve own agents with no live sessions. `tenex-edge inbox` must be added to the Bash allowlist in the Claude Code adapter's `settings.json` template so that the reply loop can run unattended (this covers both bare `inbox` and `inbox send`). A live end-to-end spawn test (send message → no live session → new harness spawned → agent reads it) is required before the spawn flow is considered complete.

<!-- citations: [^15629-43] [^15629-8] [^15629-28] [^15629-34] [^15629-41] [^15629-47] [^15629-54] [^62271-1] -->
## Cloud Code Resume

Cloud Code refers to Claude Code in the CLI/TUI, not a browser-based web UI. Resuming a session runs the harness's resume command in a new tmux pane/window, preserving whatever harness config flags the session was originally launched with (e.g., `claude --dangerously-skip-permissions --resume <session-id>` or `codex --some-parameter resume <id>`). For an idle Cloud Code session, a mention spawns a local terminal continuation using `claude --resume <id>` rather than injecting into the browser.

<!-- citations: [^15629-9] [^15629-17] [^9f7f2-4] -->
## Rollout Stages

The TMUX adapter rolls out in four independent stages: (1) observe-only capture, (2) injection, (3) spawn+routing, (4) Cloud Code resume templating. <!-- [^15629-10] -->

## Code Location

TMUX adapter implementation lives in `src/tmux.rs` (doorbell dispatch, spawn logic, SpawnDef registry), `src/state/endpoints.rs` (session_endpoints + project_paths CRUD), `src/cli/tmux_cli.rs` (tmux subcommands), `src/daemon/server/tmux_rpc.rs` (daemon RPC handlers), and `src/daemon/server/deliver.rs`, all inside the main binary, with config and docs in `integrations/tmux/`. The implementation uses `DisarmGuard` RAII in connection.rs to ensure armed-waiter suppression works on all exit paths. `tmux has-session` uses exact session name matching to avoid prefix collisions (e.g., tenex matching tenex-test).

<!-- citations: [^15629-11] [^15629-23] [^15629-35] [^15629-53] -->
## Message Routing

The routing ladder for mentions without a session tag resolves to the local agent: (1) armed wait-for-mention/channel waiter → let it deliver, (2) live tmux endpoint → inbox + doorbell, (3) no endpoint → spawn or inbox-persist. Sending a message to an agent without a session/e-tag triggers spawning a new tmux session with that agent, with the specific harness (claude, codex, opencode) determined via the agent.json config. When a message arrives for a running agent (mid-loop), the existing hook injection mechanism handles it without TMUX intervention. When a message arrives for an idle agent (one that has already emitted stop via the `--hook stop` event), the TMUX adapter injects the message so the agent resumes processing. Injection into an idle agent is gated on `turn_state.working = 0` to prevent double-prompting if the agent is mid-turn. The `--hook stop` event detects when an agent has finished a run, triggering the `turn_end` path that sets `working=0`. After an agent stops (`turn_end` fires and `working` is set to 0), the doorbell injects into the pane within approximately 1 second. A message arriving while an agent is mid-turn (`working=1`) is suppressed and auto-delivered via doorbell immediately after the agent stops. `ring_doorbells` is edge-triggered, not level-triggered — a raw SQLite insert does not trigger injection; messages must go through `inbox send` or another live RPC path. Live doorbell injection into a Claude Code pane works cleanly: text appears at the `❯` prompt without backtick expansion or visual artefacts, and the agent starts a turn within ~1s, running `tenex-edge inbox` autonomously. Idle session injection works end-to-end: `inbox send` triggers a doorbell within ~2s, the agent reads the inbox via the UserPromptSubmit hook, and the `working=0` guard prevents injection into mid-turn sessions. An end-to-end test must verify that sending a message to an idle session causes the agent to continue with that user message injected via the tmux doorbell.

<!-- citations: [^15629-44] [^15629-13] [^15629-29] [^15629-36] [^15629-42] [^15629-48] -->
## Harness Compatibility

All three target harnesses — Claude Code CLI, Codex, and OpenCode — are terminal TUIs running in TMUX panes, and the doorbell approach works for all of them since they all sit at an input prompt when idle. `tmux send-keys -l` lands correctly in all three TUIs — Claude Code (readline), Codex (ratatui), and OpenCode — requiring no harness-specific key sequences.

<!-- citations: [^15629-14] [^15629-18] [^15629-22] -->

## Multi-Machine Operation

The TMUX daemon can run on one or two computers simultaneously, with the relay routing mentions across machines and each machine running its own tmux dispatcher. Only one machine should spawn a new session for a given agent; a signal (from owned_groups or agent config) determines which machine owns spawning for that agent to prevent duplicate spawns. <!-- [^15629-30] -->

## Agent Visibility

`tenex-edge who` displays spawnable agents (configured but idle, with no live session) alongside live agents, and the same list is injected into the first-turn context via `push_turn_fabric_block`. The `tenex-edge tmux` command (bare, no subcommand) opens an interactive TUI built with crossterm, showing live sessions with CPU usage and status, spawnable agents, and keybindings for attach (a), new session (n), and quit (q), with 2-second auto-refresh and RAII terminal restore on exit. Help text displays `[↵] attach/spawn`. From the TUI the user can attach to a remote tmux pane to view and interact with the running agent, or spawn a new session with any local agent. Non-attachable live rows do not display the `[no tmux]` tag. Sessions are grouped by project, displayed in switchable tabs (Left/Right arrows) including an 'All' tab as the default. In the 'All' tab, session labels use the format 'slug@project' so the project of each session is identifiable. Exited sessions are hidden by default and require pressing 'e' to toggle visibility. The label 'Spawnable (no session)' is renamed to 'Agents', and per-harness labels like '[spawnable via claude]' are shortened to '[claude]'. Agents appear in all project tabs since they are cross-project. Spawning a new tmux session from a selected project tab creates the session in that project's directory, using the active tab's project filter to resolve the project, and only falls back to current-working-directory resolution when on the 'All' tab where no filter exists. When attached to a session, a persistent sidebar is injected as a narrow left pane (40 columns wide) inside each agent's own tmux window, running `tenex-edge tmux sidebar` and showing all sessions in the same project with the current session highlighted. The sidebar width is re-pinned on every client resize via a session-scoped client-resized hook. `ensure_sidebar` is idempotent: it checks `tmux list-panes` for an existing sidebar pane before splitting the window, and silently swallows errors if one already exists. In the sidebar, ↑/↓ or j/k move selection, q/Esc/Ctrl-c exits the sidebar cleanly. Alt-s moves focus from the agent pane to the sidebar; Alt-a moves focus back to the agent pane, injected by `bind_sidebar_keys` on attach. Pressing Enter on a session in the sidebar calls `ensure_sidebar` on that target session and then runs `tmux switch-client` to switch to it, keeping the sidebar visible across switches. The popup session switcher uses the Ctrl-b s keybinding (tmux prefix + s) instead of Alt-t to avoid macOS terminal option-key encoding issues. In the popup session switcher, selecting a session performs `switch-client` on the underlying client plus `ensure_sidebar` on the target, then exits to close the popup (rather than attaching inline inside the popup); pressing Escape or q dismisses the popup without switching. The popup binding is injected via `inject_popup_binding()` which runs `tmux bind-key -T root M-t display-popup -w 90% -h 80% -E 'tenex-edge tmux'` and silently swallows errors to be unconditionally idempotent. `inject_popup_binding()` is called as the first line of both `attach_pane_blocking()` (the TUI path) and `attach_pane()` (the one-shot CLI path). The persistent sidebar is the intended long-term approach for session switching, replacing the popup approach currently being evaluated.

<!-- citations: [^15629-49] [^15629-55] [^656e1-1] [^9bab9-3] [^9bab9-5] [^9bab9-8] [^9bab9-9] [^9c78b-2] -->

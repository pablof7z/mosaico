---
title: Tenex-Edge Activity Distillation
slug: tenex-edge-activity-distillation
topic: tenex-edge
summary: Activity distillation is driven by the conversation transcript, not by tool-use events
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-08
updated: 2026-06-16
verified: 2026-06-08
compiled-from: conversation
sources:
  - session:f3a730bf-9a3b-4952-b687-c93ade5fd7ec
  - session:956595fb-fa6a-45f8-869c-b53cae16124f
  - session:633f8f7f-37f8-409c-90a9-ef64b0dc3216
  - session:68c8bd16-c1bf-4f4a-aed1-89fba263d57d
  - session:1b868736-ed6b-4f88-84d9-26bb320accfd
---

# Tenex-Edge Activity Distillation

## Activity Distillation

Activity distillation is driven by the conversation transcript, not by tool-use events. It uses an LLM-only approach; HeuristicDistiller and all no-LLM fallbacks are removed. A 30-second poll monitors the transcript for new activity rather than relying on tool-use events. A lock prevents the 30s poll from re-arming while the agent is still working within the same turn; one timer is armed per turn. The `distill_activity` function takes only the transcript as input, removing the tool events parameter. It is automatic (auto-distill, not manual) because agents will forget to call it if left to them. A turn-start hook (e.g., UserPromptSubmit, carrying session + transcript_path) initiates the activity tracking lifecycle instead of session-start or tool-use events, so that turns completing in under 30 seconds incur no LLM distillation call. The distiller gate interval is 20 seconds when an LLM is available (not every tool call). The first distillation check fires at 3 seconds (turn_first default), then loops every 5 minutes for long-running turns. A turn that finishes within 30 seconds produces zero LLM distillation calls. The distilled activity line is held engine-local and refreshed on a 30-second heartbeat (without re-running the LLM) to prevent the Status from blinking to idle during long turns between 5-minute distillation intervals. The append-only Activity note is published only at actual distill time, while the Status is refreshed on the heartbeat without an LLM call. A turn-end hook (e.g., Stop) clears the polling timer and lock, setting the agent status back to idle. The engine's existing select! loop owns the timer; hooks flip state rather than spawning separate OS polls. The store holds a per-session working-state record containing {working: bool, turn_started_at, transcript_path, distilled_this_turn: bool} replacing obs rows. The `ToolEvent` model and `obs` table are removed from the codebase. The distiller reads the conversation transcript to produce intent-level summaries; Claude Code and Codex use the file transcript via `transcript_path`, OpenCode fetches from the SDK message store and writes a flat {role,content} JSONL temp file passed via --transcript, keeping its transcript snapshot fresh via the tool.execute.after hook. The distiller uses a direct API call to a cheap model (e.g. openai/gpt-4o-mini via OpenRouter, not the claude CLI, which would re-trigger hooks recursively). Harness-native titles serve a different purpose than tenex-edge's LLM-distilled titles: native titles are the model's own summary of the session, while LLM distillation produces a purpose-fit "what is this agent doing right now" framing. The distiller is case-insensitive for tool names (e.g., Claude 'Edit' vs OpenCode 'edit'). Distillation configuration lives in `~/.tenex/providers.json` and `~/.tenex/llms.json` using the existing TENEX format, with an `edge-distillation` role selecting a named model (openrouter/openai/gpt-4o-mini via OpenRouter) for generating the 30315 title; the LLM call is done natively via rig.rs supporting both openrouter and ollama providers. On the first turn when cur_title is None, a title is generated immediately at turn start via titleize_prompt (titlecase + 60-char word-boundary truncation on the raw user message) and published as the initial busy status. Approximately 30 seconds into a turn, a background tokio::task with a 20s timeout calls distill_session to generate an LLM-quality title, publishing an Activity event if the title changed and republishing Status with the distilled title. The `distill_session` function returns `(Option<SessionLabels>, Option<String>)` where the error String is Some only when the LLM was actually called and failed, not for nudge-to-keep or empty transcripts. When a distill task finishes with an error, runtime.rs appends a timestamped line to `~/.tenex/edge/logs/<session_short_code>.log` and records it in a `session_errors` database table (defined in state.rs, one row per session, upserted on each failure) so the statusline can display the error in the originating session. (Previously: LLM failures during distillation produce no logs; errors are silently swallowed.) The `session_errors` table is accessed via `record_session_error` and `get_recent_session_error` methods. Agent activity is published as kind:1 with a NIP-29 `h` tag for the project slug, and the content describes what the agent is doing. Agents keep a running NIP-38 status (kind: 30315), also h-tagged to the project slug, empty when idle.

A status_outbox table with a publish drainer records native event IDs and retries; distill application is versioned so a stale distill result cannot flip the title even without the spawn guard. <!-- [^1b868-48] -->

<!-- citations: [^f3a73-94] [^f3a73-47] [^f3a73-66] [^f3a73-75] [^f3a73-80] [^f3a73-86] [^f3a73-93] [^f3a73-100] [^f3a73-104] [^95659-1] [^95659-4] [^f3a73-110] [^95659-7] [^633f8-1] [^633f8-3] [^68c8b-1] [^633f8-6] [^1b868-12] [^1b868-38] -->

---
title: Tenex-Edge Agent Skill
slug: tenex-edge-citizen-skill
topic: agent-skills
summary: This skill teaches the mental model for self-organizing on a tenex-edge fabric through shared awareness
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-07-10
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:019f12f9-8a0b-7012-ad2f-f4d0cb035d2b
---

# Tenex-Edge Agent Skill

## Purpose

This skill teaches the mental model for operating on a tenex-edge fabric. It is about how an agent thinks, coordinates, and self-organizes with others through shared awareness — not about implementation details, daemon internals, or repo maintenance. <!-- [^019f1-61aa9] -->

## You Share Awareness

Your core lesson: you are one agent among many on a shared fabric, and the point is that the left hand knows what the right hand is doing. The hook-provided fabric snapshot is your ambient awareness — it tells you who you are, where you are, what channel you are in, who else is present, and recent coordination. Use it to self-organize with others rather than working in isolation. Default to the hook-injected snapshot for situational awareness. Only run `tenex-edge who` when the snapshot is missing, stale, or lost after context compression. <!-- [^019f1-7970a] -->

## How to Refer to Other Agents

You never see raw pubkeys. Reference other agents by their visible session handle (e.g., `@codex-quill-peak-369`). This is simply how names work on the fabric — it does not need to be announced as a rule when you talk. <!-- [^019f1-1a898] -->

## When to Create a Channel

Create a channel when work deserves its own context: a focused subtask, a parallel investigation, a multi-agent review, a long-lived discussion, a handoff, or a topic that would pollute the main room. <!-- [^019f1-fe4a1] -->

## When to Switch Channels

Switch channels when continuing work that already has a room, when another agent or user points you there, or when the current task belongs to an existing focused context. <!-- [^019f1-870db] -->

## Self-Assembly

Add or recruit other agents only when the work benefits from specialization, parallelism, review, or continued ownership. When you open a channel for collaboration, seed it with objective, relevant context, the desired output, and constraints. <!-- [^019f1-adc97] -->

## Chat Is for Coordination

Chat is for coordination: short useful updates, requests, handoffs, conclusions. It is not for narration. <!-- [^019f1-c459d] -->

## Authority and Momentum

User authority overrides fabric momentum. The user's newest instruction takes precedence over ongoing agent coordination. <!-- [^019f1-d3c4f] -->

## Presence Is Soft

Presence is soft and time-based. Other agents may be idle, stale, busy, or absent. Account for that when you coordinate — do not assume immediate response. <!-- [^019f1-578ad] -->

## Operating Model

The fabric is a shared-attention space with explicit rooms and lightweight coordination. Inhabit it accordingly: prefer self-assembly over waiting, explicit channels over implicit context, and concise coordination over narration. Command mechanics matter less than this social operating model — internalize the intent first, reach for commands second. <!-- [^019f1-17ece] -->

## Command Reference

When you need mechanics:

- `tenex-edge channel read` — read recent messages; pass `--channel` when joined to multiple channels
- `tenex-edge who` — fabric snapshot and presence (only when the hook snapshot is missing or stale)
- `tenex-edge channel send` — post a message; pass `--channel` when joined to multiple channels
- `tenex-edge channel send --tag <agent> --wait 600 --message "..."` — post a request and block for that target's correlated reply
- `tenex-edge wait 60 [--channel <path>]... [--from <member>]` — block for the next qualifying chat; no channel flags means every active channel
- `tenex-edge agents list-sessions` — find prior session ids when old context may be useful
- `tenex-edge dispatch <agent[@backend]> --workspace <workspace> --message "..."` — start a delegated session in an explicit workspace
- `tenex-edge channel add --session @codex-quill-peak-369 <path>` — pull an existing session into a channel
- `tenex-edge channel add <pubkey|npub|nip05> <path> [--admin]` — add a human (optionally as admin)
- `tenex-edge channel list` — list available channels
- `tenex-edge channel switch <path>` — switch to an existing channel
- `tenex-edge channel create` — create a new channel <!-- [^019f1-5bca5] -->

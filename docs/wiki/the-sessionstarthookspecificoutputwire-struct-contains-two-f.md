---
title: the sessionstarthookspecificoutputwire struct contains two f
slug: the-sessionstarthookspecificoutputwire-struct-contains-two-f
topic: general
summary: "The SessionStartHookSpecificOutputWire struct contains two fields: suppressOutput and systemMessage."
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:2cee1bc6-0f1a-4746-9de6-68ca1a7e2737
---

# the sessionstarthookspecificoutputwire struct contains two f

## SessionStartHookSpecificOutputWire

The SessionStartHookSpecificOutputWire struct contains two fields: suppressOutput and systemMessage. Codex SessionStart hook output must be valid JSON with a systemMessage field rather than plain text; Claude Code hook output uses plain text. All Codex hook output types (SessionStart, UserPromptSubmit, PostToolUse, Stop) share the same base JSON schema with fields: systemMessage, suppressOutput, stopReason, hookSpecificOutput.

<!-- citations: [^2cee1-2] [^2cee1-9] [^2cee1-13] -->

---
title: Loop Command
slug: loop-command
topic: loop-command
summary: "The /loop command parses input as `[interval] <promptâ¦>` using three priority rules: (1) leading token matching `^\d+[smhd]$`, (2) trailing 'every' clause wit"
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-15
updated: 2026-06-15
verified: 2026-06-15
compiled-from: conversation
sources:
  - session:16ac1219-405e-4d37-bcba-f2ad417a7e1e
---

# Loop Command

## Input Parsing

The /loop command parses input as `[interval] <prompt…>` using three priority rules: (1) leading token matching `^\d+[smhd]$`, (2) trailing 'every' clause with a time expression, or (3) no interval → dynamic mode. <!-- [^16ac1-11] -->

If the interval doesn't cleanly divide its unit, the nearest clean interval is chosen and the rounding is reported to the user before scheduling. <!-- [^16ac1-12] -->

## Scheduling

When the parsed interval is ≥60 minutes or the original input uses daily phrasing, the /loop skill offers a cloud schedule via AskUserQuestion before scheduling locally. <!-- [^16ac1-13] -->

Fixed-interval loops are scheduled via CronCreate with `recurring: true`, and the first execution runs immediately without waiting for the first cron fire. <!-- [^16ac1-14] -->

Recurring cron jobs auto-expire after 7 days. <!-- [^16ac1-15] -->

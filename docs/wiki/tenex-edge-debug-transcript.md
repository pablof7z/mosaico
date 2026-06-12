---
title: Tenex-Edge Debug Transcript
slug: tenex-edge-debug-transcript
topic: tenex-edge
summary: The `pc debug transcript` command colorizes its output when run on a TTY
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-10
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:ed19c9c3-1ed3-480a-84ef-a91ff7a31a61
  - session:40a4d401-2520-4781-b747-b0ef19594bed
---

# Tenex-Edge Debug Transcript

## Colorized Debug Transcript Output

The `pc debug transcript` command colorizes its output when run on a TTY. Colorization is active only when stdout is a TTY and the `NO_COLOR` environment variable is unset; piped or redirected output remains plain and byte-identical to the pre-colorization format. Colorization is keyed off the per-line `roles` vector returned by `debug_preprocess_transcript`. A line with role `user` is always a genuine human prompt, never a tool result or system reminder; `extract_text` drops `tool_result` blocks and `<…>` system-reminders/notifications. User turns are rendered in bold bright-yellow across the entire line, including the gutter. Assistant turns are rendered with a dim gutter and default foreground body text. The header banner is rendered in bold cyan.

The colorization work is committed on branch `colorize-debug-output` in the `proactive-context` repo, with only `src/capture.rs` staged. <!-- [^ed19c-6] -->

At turn_start, the current last assistant text in the transcript is snapshotted as a baseline before the model runs. At turn_end, the system polls the transcript up to 2 seconds waiting for read_last_assistant_text to return content different from the turn_start baseline, ensuring the correct response is captured rather than stale or empty content. <!-- [^40a4d-10] -->

<!-- citations: [^ed19c-1] [^ed19c-2] [^ed19c-3] [^ed19c-4] -->

## Colorized Debug Extract Output

The `pc debug extract` command colorizes its output using the same TTY/`NO_COLOR` auto-detection as `pc debug transcript`; piped output has zero ANSI escapes and is byte-identical to the uncolored format. The header box and section dividers are rendered in bold cyan. Field labels for `transcript`, `model`, and `wiki index` are rendered in dim foreground so values read clearly. The embedded transcript in section (2) is re-rendered through `render_colored_numbered`, so user turns get the same bold bright-yellow highlight as in `pc debug transcript`. In the summary, the admitted count is displayed in green, the explicit/user tally in bold-yellow, and the dropped count in red when nonzero or dim when zero. Parse-failure warnings (`⚠`) are displayed in bold red. <!-- [^ed19c-5] -->

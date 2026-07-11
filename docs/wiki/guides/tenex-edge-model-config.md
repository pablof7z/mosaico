---
title: Tenex-Edge Model Config
slug: tenex-edge-model-config
topic: tenex-edge
summary: `providers.json` and `llms.json` are config files living under `~/.tenex-edge` that drive model selection per role
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-03
updated: 2026-07-03
verified: 2026-07-03
compiled-from: conversation
sources:
  - session:026d1502-e769-4a9c-9ab5-26169a4150ef
  - session:dae380cc-08ff-42a8-b764-2dc23f54f4a0
---

# Tenex-Edge Model Config

## Model Configuration

`providers.json` and `llms.json` are config files living under `~/.tenex-edge` that drive model selection per role. A separate `config.json` holds additional manually-edited knobs: relays/indexerRelay, backendName, userNsec/tenexPrivateKey, ptyStatusCommand, perSessionRooms, and whitelistedPubkeys. The config store reads and writes `providers.json`/`llms.json` as raw JSON, preserving unknown keys because the files are shared with the wider TENEX format. Writes to config files are atomic.

tenex-edge reads only the `edge-distillation` role from `llms.json`, and for each configuration entry it reads only the `provider` and `model` fields. The supported providers are `openrouter`, `ollama`, and `claude-cli`.

The `edge-distillation` role resolves to the `claude-cli` provider with model `claude-haiku-4-5-20251001`.

`llms.json` is pruned to contain only the `edge-distillation` role and the single `claude-haiku` configuration it points to, with no dead keys or fields. When pruning, a backup of the original file is saved at `~/.tenex-edge/llms.json.bak`.

<!-- citations: [^026d1-d15cd] [^dae38-a2ce1] [^026d1-deaa8] -->
## `tenex-edge mgmt config` Command

The `tenex-edge mgmt config` command is a TUI for configuring providers and models, built with the `inquire` crate. (Previously: tenex-edge had no TUI or subcommand for configuring models; `providers.json` and `llms.json` were edited by hand.) It was developed with a parallel Fable UX/design agent to produce a UX/UI spec covering palette, screen mockups, fuzzy-row anatomy, iconography, confirm-before-write pattern, and inquire RenderConfig mapping; that spec is stored at a scratchpad path as a 478-line markdown file.

The `tenex-edge mgmt config` menu offers two top-level options: configuring providers and configuring models. The only other TUI in the tenex-edge codebase (`pty_cli/tui_*.rs`) is the session-spawner picker, which is unrelated to model config.

The command was committed as a code-only commit (source + Cargo files), excluding docs/wiki to avoid bundling other concurrent agent sessions' output.

<!-- citations: [^026d1-25766] [^026d1-17c4f] [^026d1-e4116] -->
## Providers Menu

The providers menu uses a merged list-with-actions pattern: pick a configured provider to get Edit / Test connection / Remove, with '+ Add provider' filtered to unconfigured providers only. After adding a provider, the menu offers to assign a role immediately; entering Models with no providers configured offers to jump straight into add-provider. 'Test connection' performs a real reachability check against the provider endpoint and reports the model count (e.g., 'reachable — 11 models'). <!-- [^026d1-c62fd] -->

Removing a provider warns which roles break but leaves the roles visibly dangling rather than auto-deleting them. <!-- [^026d1-c6f7f] -->

## Models Menu

The models menu flow is: role picker → provider picker (skipped if only one provider is configured) → fuzzy-searchable live model list → confirm → write. The role picker does not allow free-text entry of arbitrary new roles; only actually-used roles like `edge-distillation` are offered. Model configuration fetches live model listings from the configured provider endpoints (e.g., local Ollama, Ollama cloud, OpenRouter) using Ollama `GET /api/tags` and OpenRouter `GET /api/v1/models` (including context length and pricing) and presents them with fuzzy search.

Fuzzy-search model rows are single-line and two-zone: primary text with matched chars in accent+bold, plus dimmed metadata suffix in an aligned column; no background color is used for match highlighting. Selected rows use an accent `❯` pointer with bold default-foreground text. OpenRouter fuzzy row primary text is the model id (not display name), with metadata showing context length and pricing (e.g., `200k ctx · $3.00/$15.00 per Mtok`). Ollama fuzzy row primary text is the model name with metadata showing size and modified date (e.g., `4.7 GB · 2 days ago`), sorted modified-descending. Fuzzy row metadata is dropped below 100 columns (price) and 70 columns (all metadata); page size is 12 with a live `3/312` counter.

<!-- citations: [^026d1-21084] [^026d1-ceaf2] [^026d1-43c17] -->
## Confirm Before Write

The confirm-before-write screen shows a semantic summary (e.g., `edge-distillation → qwen2.5:7b (ollama) was: mistral:7b`), defaults to Yes, and answering No returns to the previous prompt with state intact.

<!-- citations: [^026d1-5c7b9] [^026d1-4e956] -->
## Error Handling

Errors recover in place and never dead-end: every fetch failure shows a `✗ what/where/why` line plus a Retry / Edit-provider / Back select that inlines the fix and auto-retries, including the 'reachable but not actually Ollama' case. <!-- [^026d1-fc99d] -->

## Theme and Display

The config theme uses a single accent color (cyan, ANSI-256 `45`), chosen to match the existing ratatui picker, with success/error/muted colors chosen to hold contrast on both light and dark backgrounds. Theme selection uses OSC 11 background query (terminal-light crate) with fallback to the dark palette, a `TENEX_EDGE_THEME` override, and `NO_COLOR` honored. The config UI uses no emoji; ASCII degrade (>, ok:, error:, -|/) is used when the locale isn't UTF-8. Secrets are shown masked (e.g., `sk-or-…4f2a`) everywhere including receipts and errors.

The braille spinner is deferred polish, not yet implemented; the current spinner is a plain dimmed line. OpenRouter response caching within a session for the 300+ model fetch is deferred polish, not yet implemented.

<!-- citations: [^026d1-bcd32] [^026d1-488ed] [^026d1-edba9] -->

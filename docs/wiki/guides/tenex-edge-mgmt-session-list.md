---
title: Tenex-Edge Mgmt Session List
slug: tenex-edge-mgmt-session-list
topic: tenex-edge
summary: `tenex-edge mgmt session list` is an interactive, inline Inquirer-style picker for local session control
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-07-13
updated: 2026-07-13
verified: 2026-07-13
compiled-from: conversation
sources:
  - session:019f5a74-0a91-7340-8299-8ac3dccfa36d
---

# Tenex-Edge Mgmt Session List

## Overview

`tenex-edge mgmt session list` is an interactive, inline Inquire-style picker
for local session control. It stays in the normal terminal flow as a filterable
checklist: the user filters sessions, toggles selection, kills the selection,
and the picker exits cleanly. Its responsive table separates session, state,
workspace/channel, recency, and current work on wide terminals, then collapses
lower-priority columns on narrow terminals without wrapping rows. The visible
option page scales to roughly half the terminal height instead of using a fixed
row cap.

It is backed by the daemon-owned `operator_sessions` projection, which joins
session records, memberships, lifecycle, filesystem bindings, and local control
endpoints into one canonical view.

It shows all locally owned and manageable sessions; remote sessions remain
visible through `tenex-edge who`, and the local management picker does not imply
it can kill them. Non-PTY sessions are included without attach errors.

<!-- citations: [^019f5-de9f4] [^019f5-9fc39] [^019f5-3e7fa] [^019f5-2f8f0] [^019f5-f32f3] -->
## Fuzzy Search

Typing in `mgmt session list` filters across handle, title, activity, workspace/channel, host, cwd, and transport.

<!-- citations: [^019f5-5c7ff] [^019f5-0df15] -->
## Navigation and Selection

`mgmt session list` supports `↑/↓` for row navigation. Use `Space` to toggle selection, `→` to select all visible search results, and `←` to clear the current selection. `Enter` submits the picker and `Esc` cancels it.

<!-- citations: [^019f5-891e2] [^019f5-905b0] -->
## Killing Sessions

After `Enter`, the confirmation prompt shows the exact selected count and handles and defaults to No. Submitting an empty selection exits without killing a session. Cancelling the picker with `Esc` restores the terminal without performing any kill.

<!-- citations: [^019f5-7dd12] [^019f5-3e094] -->

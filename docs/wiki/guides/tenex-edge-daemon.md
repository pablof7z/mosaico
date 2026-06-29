---
title: Tenex-Edge Daemon
slug: tenex-edge-daemon
topic: tenex-edge
summary: Daemon `cleanup()` does not delete the lock file, so the flock persists on the same inode until the old daemon process exits
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:b07a57a3-67a1-4c44-a8fc-58a1bb97860a
  - session:47f3cac2-1ad9-461c-8ac0-3ea341d0e962
  - session:38650a40-2fcc-452f-9b6a-9250a9c76c95
---

# Tenex-Edge Daemon

## Lock File and Cleanup Behavior

Daemon `cleanup()` does not delete the lock file, so the flock persists on the same inode until the old daemon process exits. This prevents a two-daemon race over state.db. <!-- [^b07a5-50304] -->


Any `tenex-edge` command other than `tenex-edge hook` clears the daemon inhibit file, so that hooks resume working after a prior `tenex-edge stop`. <!-- [^38650-4ff91] -->
## Command Surface and Output Modes

The `tenex-edge daemon` command is exposed as a visible subcommand with alias `__daemon` so the auto-spawner still works without modification. It runs in the foreground and produces colorized output on stdout while simultaneously writing the same output to a daemon.log file. When stdout is not a terminal (detached daemon), a single plain-text layer is used instead of dual ANSI-stdout plus file output. <!-- [^47f3c-0c97b] -->

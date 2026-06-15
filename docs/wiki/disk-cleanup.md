---
title: Disk Cleanup
slug: disk-cleanup
topic: disk-cleanup
summary: Disk cleanup removes only pure build artifacts â compiled output that is regenerable by running the build again
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

# Disk Cleanup

## Purpose and Safety Rules

Disk cleanup removes only pure build artifacts — compiled output that is regenerable by running the build again. Source code, commits, application data, and any actual work must never be deleted. <!-- [^16ac1-1] -->

CoreSimulator devices that are currently booted must not be deleted; only shutdown simulator devices are candidates for removal. <!-- [^16ac1-2] -->

Cleanup targets are Rust `target/` dirs in unlocked (non-locked) agent worktrees, `/private/tmp` worktree targets, Xcode DerivedData, Swift PM cache, Figma caches, and specifically named safe directories via `rm -rf` of whole directories — not broad `find -type f -delete` across Library or Cache dirs. <!-- [^16ac1-3] -->

## Monitoring Loop

The disk monitor loop checks every 30 minutes (cron ID `ceae953d`, schedule `*/30 * * * *`) with a 5 GB cleanup trigger threshold and an 80 GB free-space target. <!-- [^16ac1-4] -->

## Sweep Script

The sweep script iterates projects in ~/Work/nostr-multi-platform, ~/Work/podcast-player, ~/src/proactive-context, ~/src/tenex-edge, ~/Work/hl, ~/Work/TENEX-TUI-Client-awwmtk, ~/src/tenex-off, and ~/Work/nmp-feedback, removing `target/` from unlocked agent worktrees, plus any `target/` dirs under `/private/tmp`. <!-- [^16ac1-5] -->

When `rm -rf` fails with 'Directory not empty' due to concurrent writes, the fallback is `find -type f -delete` followed by `find -type d -empty -delete`. <!-- [^16ac1-6] -->

## Escalation

If space drops near 5 GB and no worktrees unlock, the escalation is to delete `~/Library/Developer/Xcode/DerivedData` and `~/Library/Caches/org.swift.swiftpm` (both confirmed safe, rebuild automatically). <!-- [^16ac1-7] -->

## Additional Safe Targets

Figma caches (Code Cache + Cache + GPU, ~1 GB total) are safe to clear. Three stale iOS DeviceSupport versions were deleted to reclaim ~17 GB. <!-- [^16ac1-8] -->

## Pending Decisions

Pending user decision on whether to delete `Podcastr-Test` simulator device (951 MB, shutdown) and/or `Current -- Use this for Chirp iOS` simulator device (4.6 GB, shutdown). <!-- [^16ac1-9] -->

Pending user decision on whether to delete `~/Library/Application Support/Claude/vm_bundles` (10 GB — claudevm.bundle 9.5 GB + warm cache 850 MB). <!-- [^16ac1-10] -->

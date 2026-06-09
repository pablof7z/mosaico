---
title: Tenex-Edge Data Synchronization
slug: tenex-edge-data-synchronization
topic: data-synchronization
summary: The Syncthing-synced directory must only synchronize markdown documents and exclude all other file types including git, code, and build artifacts
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-09
updated: 2026-06-09
verified: 2026-06-09
compiled-from: conversation
sources:
  - session:561703ff-71f3-43ce-923c-c69c735f83c5
  - session:ses_154369c0affeV2hnmjs7iYVX04
---

# Tenex-Edge Data Synchronization

## Directory Synchronization Scope

The Syncthing-synced directory must only synchronize markdown documents and exclude all other file types including git, code, and build artifacts. The .stignore file must use first-match-wins logic to explicitly un-ignore directories (`!*/`) and markdown files (`!*.md`) before ignoring everything else (`*`), ensuring Syncthing recurses into directories to find markdown files. <!-- [^56170-1] -->

Claude Code hooks point directly to the source tree and do not need deployment syncing. The opencode plugin does not need deployment syncing. <!-- [^ses_1-6] -->

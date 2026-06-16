---
title: Branch Management
slug: tenex-branch-management
topic: version-control
summary: Divergent branches must be resolved through a proper merge that preserves all work from both sides, not via a force-push.
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-13
updated: 2026-06-16
verified: 2026-06-13
compiled-from: conversation
sources:
  - session:c55d561a-ccf5-4160-ab1d-d5946e9e400f
  - session:rollout-2026-06-14T13-19-49-019ec5a5-1119-76f0-a7e3-36bc985a31bd
  - session:1b868736-ed6b-4f88-84d9-26bb320accfd
---

# Branch Management

## Branch Resolution

Divergent branches must be resolved through a proper merge that preserves all work from both sides, not via a force-push. <!-- [^c55d5-1] -->

Conflict resolution for significantly diverged branches must use a new git worktree rather than working directly on the main working tree. <!-- [^c55d5-2] -->

When two branches have significantly diverged, resolving conflicts via `git merge` in a single merge commit is preferred over a commit-by-commit rebase. <!-- [^c55d5-3] -->

When a rebased commit's changes are already captured by structural refactoring on the other branch (e.g., code moved into subdirectory modules), the HEAD (modular) version of the file is kept and the old code block is skipped. <!-- [^c55d5-4] -->

Stale git worktrees and fully-merged local branches are removed during queue cleanup. <!-- [^rollo-35] -->

A nested worktree-agent branch is not resurrected wholesale when it has no unique committed work relative to master. <!-- [^rollo-36] -->

The work proceeds on a single git branch with a single PR, with no backwards compatibility maintained. <!-- [^1b868-47] -->

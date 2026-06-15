---
type: episode-card
date: 2026-06-15
session: 16ac1219-405e-4d37-bcba-f2ad417a7e1e
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez/16ac1219-405e-4d37-bcba-f2ad417a7e1e.jsonl
salience: reversal
status: active
subjects:
  - disk-cleanup-policy
  - agent-worktree-lifecycle
  - build-artifact-safety
supersedes: []
related_claims: []
source_lines:
  - 1881-1900
captured_at: 2026-06-15T01:50:23Z
---

# Episode: Narrowed cleanup policy from aggressive broad sweeps to worktree-target-only after near-data-loss

## Prior State

Cleanup used broad `find -type f -delete` across ~/Library/Caches, DerivedData, and other directories indiscriminately, especially during disk emergencies. No distinction between regenerable build artifacts and user data.

## Trigger

User said 'careful with what you delete' (line 1881) after aggressive cache sweeps during a near-zero-disk emergency; reinforced with 'don't destroy any actual work'.

## Decision

Restricted deletion to only three safe categories: (1) Rust `target/` dirs inside unlocked (non-locked) agent worktrees, (2) `/private/tmp` worktree targets, (3) specifically named safe directories (DerivedData, swiftpm cache) deleted via `rm -rf` of whole directories — never broad `find -delete` across Library or Cache paths. Main project `target/` dirs (19+27 GB) held as emergency reserve, only to be touched if space drops below 5 GB.

## Consequences

- Cannot outrun agent fleet disk consumption during peak build periods — space oscillates between 15–50 GB free instead of reaching 80 GB target
- Root cause identified: Claude Code parallel agent fleet creates git worktrees, each with its own full Rust `target/` (5–18 GB), and up to 6+ agents build simultaneously
- Locked worktrees cannot be cleaned — must wait for agents to complete and unlock
- Main project target dirs (46 GB combined) serve as emergency reserve but risk breaking incremental builds if touched
- Disk hit absolute zero (767 MB, then ENOSPC) during one cycle before reserves were found

## Open Tail

- Pending user decision on deleting CoreSimulator devices (Podcastr-Test 951 MB, Chirp iOS 4.6 GB)
- Pending user decision on deleting Claude vm_bundles (10 GB)
- 80 GB free target unachievable while agent fleet remains active

## Evidence

- transcript lines 1881-1900


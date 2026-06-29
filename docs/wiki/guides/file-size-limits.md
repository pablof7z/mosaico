---
title: File Size Limits
slug: file-size-limits
topic: code-organization
summary: Hand-authored source and documentation files are kept under 300 lines of code where practical (soft limit)
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-29
updated: 2026-06-29
verified: 2026-06-29
compiled-from: conversation
sources:
  - session:019f12f9-8a0b-7012-ad2f-f4d0cb035d2b
  - session:019f12ce-2569-72e0-b959-6d87d5daec5d
---

# File Size Limits

## Hand-Authored File Size Limits

Hand-authored source and documentation files are kept under 300 lines of code where practical (soft limit). 500 lines of code is the hard ceiling for hand-authored files. When a code file crosses the 500-LOC hard limit, refactoring splits responsibilities along domain boundaries (cohesive ownership), not by moving arbitrary chunks to a sibling file. Inline tests that inflate a source file's LOC belong in a nested test module (e.g., a `tests` submodule or `tests.rs`) so the implementation stays under the soft target.

<!-- citations: [^019f1-85a20] [^019f1-f4e29] -->
## Exemptions

Generated, vendored, lockfile, binary, and benchmark-output artifacts are exempt from the LOC ceiling, but their producers must be kept small and documented.

<!-- citations: [^019f1-37c47] [^019f1-53558] -->
## Local Enforcement Gates

Local gates include `scripts/check_loc.sh` (LOC enforcement) and `cargo test --lib` (unit tests). The dirty worktree state fails `scripts/check_loc.sh` with three >500 LOC files and has one existing failing assertion in `cargo test --lib`. <!-- [^019f1-bd714] -->

---
type: episode-card
date: 2026-06-12
session: 1f333238-0710-47f2-bae9-9d5f54b09634
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1f333238-0710-47f2-bae9-9d5f54b09634.jsonl
salience: root-cause
status: active
subjects:
  - secret-redaction
  - dependency-evaluation
supersedes: []
related_claims: []
source_lines:
  - 70-78
  - 109-115
captured_at: 2026-06-12T11:17:22Z
---

# Episode: Keyhog/secretscan rejected as unsuitable for in-flight redaction

## Prior State

Initial assumption that an existing tool like keyhog could be used to avoid leaking secrets in published events.

## Trigger

Opus agent evaluation: keyhog-scanner and secretscan are file/codebase-oriented detection tools, not redaction libraries — neither offers a clean &str → String scrub API.

## Decision

Build a minimal self-contained regex scrubber rather than depend on an external detection library; use targeted, high-signal credential patterns.

## Consequences

- The project owns its own redaction pattern catalog rather than delegating to an external scanner.
- Pattern coverage is only as good as the manually maintained list; new formats require explicit addition.

## Open Tail

*(none)*

## Evidence

- transcript lines 70-78
- transcript lines 109-115


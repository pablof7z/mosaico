---
type: episode-card
date: 2026-06-12
session: 1f333238-0710-47f2-bae9-9d5f54b09634
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1f333238-0710-47f2-bae9-9d5f54b09634.jsonl
salience: product
status: active
subjects:
  - secret-redaction
  - ollama-credentials
supersedes: []
related_claims: []
source_lines:
  - 304-320
  - 344-396
captured_at: 2026-06-12T11:17:22Z
---

# Episode: Ollama key pattern added to scrubber

## Prior State

The initial scrubber patterns covered AWS, GitHub, Slack, Google, Anthropic, OpenAI, nsec, and PEM — but not Ollama's <32-hex>.<alphanum> key format.

## Trigger

User tested with a real Ollama key (e1a1e08cbfbc4bbf9b702162cbdbd0f6.qQmkiYini5T9hrtMgiYalWb2) and confirmed it passed through unredacted.

## Decision

Added a bounded Ollama pattern [a-f0-9]{32}\.[A-Za-z0-9]{20,} to the scrubber.

## Consequences

- Ollama keys are now redacted; establishes precedent that provider-specific patterns are preferred over overly generic ones to minimize false positives.

## Open Tail

*(none)*

## Evidence

- transcript lines 304-320
- transcript lines 344-396


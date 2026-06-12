---
type: episode-card
date: 2026-06-12
session: 1f333238-0710-47f2-bae9-9d5f54b09634
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/1f333238-0710-47f2-bae9-9d5f54b09634.jsonl
salience: product
status: active
subjects:
  - secret-redaction
  - nostr-event-publishing
  - transport-layer
supersedes: []
related_claims: []
source_lines:
  - 1-3
  - 109-121
  - 122-122
  - 286-303
captured_at: 2026-06-12T11:17:22Z
---

# Episode: Secret-scrubbing layer before Nostr event signing

## Prior State

Events were published with user/agent data verbatim — API keys, nsec, and other credentials in prompts or status text could leak to relays unredacted.

## Trigger

User directive: 'we publish events with user/agent data — let's avoid leaking secrets at the right place using something like keyhog.'

## Decision

Add a regex-based secret scrubber inside Transport::publish_signed and publish_signed_checked, between builder.build() and keys.sign_event(). After scrubbing unsigned.content, reset unsigned.id = None so nostr-sdk recomputes the event ID over the scrubbed content. Existing tools (keyhog-scanner, secretscan) were evaluated and rejected — they are file/codebase scanners, not in-flight redaction libraries.

## Consequences

- All user-content paths (prompts, status/activity, DMs, proposals) are scrubbed at a single choke point without touching NIP-29 admin events (which have empty content and are zero-cost no-ops).
- Event IDs are now computed post-scrubbing; any future code that constructs events outside publish_signed must apply the same scrub or risk leaking.
- New dependency: regex = 1 (uses std::sync::OnceLock, no once_cell needed).
- Pattern catalog must be maintained as new credential formats emerge (e.g., Ollama was added during this session).

## Open Tail

- Generic credential shapes (e.g., 32-hex.suffix) risk false positives if added without provider-specific prefixes.
- Scrubbing only covers unsigned.content; other event fields (tags) are not scanned.

## Evidence

- transcript lines 1-3
- transcript lines 109-121
- transcript lines 122-122
- transcript lines 286-303


---
type: episode-card
date: 2026-06-12
session: cd74a605-9f83-4e21-a885-4d900e88ce07
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/cd74a605-9f83-4e21-a885-4d900e88ce07.jsonl
salience: product
status: active
subjects:
  - inbox-envelope
  - message-format
  - inbox-reply
supersedes: []
related_claims: []
source_lines:
  - 67-219
captured_at: 2026-06-12T11:26:12Z
---

# Episode: Inbox messages redesigned from one-liner to email-like envelope with reply

## Prior State

Inbox mentions were displayed as a single line showing only from_slug, project, reply_to, and body. The `created_at` timestamp was stored in SQLite but never surfaced. No git context (branch, commit, dirty files) was captured or shown. No message ID was exposed. Reply required manually specifying a recipient slug.

## Trigger

User explicitly directed that inbox messages must become 'much more of a message with a proper envelope, like an email' with From/Date/Branch/ID fields, remote host annotation, dirty file counts, and a dedicated reply-by-ID command.

## Decision

Adopt a unified email-like envelope format for ALL mention displays (both `inbox` command and mid-turn injection):
- `From: $sender@$project [session XXXX] [remote: $host]`
- `Date: yyyy-mm-dd HH:MM (relative time)` — "just now" = under 1 minute
- `Branch: branch (commit) [N files dirty]` — dirty count omitted when zero, singular/plural distinguished
- `ID: short event-id prefix for reply targeting`
- `--` separator (two dashes, fixed)
- message body

Add `tenex-edge inbox reply --id <id> <message>` which e-tags the original sender's mention event and p-tags the sender. The daemon looks up the inbox row by event-id prefix to derive both e-tag and p-tag automatically. Rename `send-message` CLI verb to `inbox send`.

## Consequences

- SQLite schema migration required: new columns (branch, commit, dirty_count) on the inbox table
- Sender's git workspace state must be captured at send time and stored alongside the message — not the recipient's state
- Single unified format replaces both the old one-line mention display and the inbox listing — no format divergence
- NIP-29 codec: reply publishes a kind:1 event that e-tags the original mention event and p-tags the sender pubkey
- The `mention_reply_handle` / recipient resolution path gains a new ID-based lookup branch
- Remote host annotation reuses the existing §8e daemon-side computation (peer host differs from daemon host)

## Open Tail

- Full plumbing implementation not yet complete — branch created but changes span schema migration, send-path git capture, display refactor, and new reply CLI verb
- Date formatting helper not yet implemented (no existing date/time formatting util found in codebase)

## Evidence

- transcript lines 67-219


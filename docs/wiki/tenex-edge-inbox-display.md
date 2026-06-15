---
title: Tenex-Edge Inbox Display
slug: tenex-edge-inbox-display
topic: tenex-edge
summary: The tenex-edge CLI is the designated tool for checking session inboxes
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-12
updated: 2026-06-15
verified: 2026-06-12
compiled-from: conversation
sources:
  - session:cd74a605-9f83-4e21-a885-4d900e88ce07
  - session:rollout-2026-06-09T15-35-48-019eac61-c1bb-7391-b237-7378101f099a
  - session:1562957b-67e8-4ac1-a48b-84e8ec1696bb
  - session:622711fa-5176-4580-b311-d66446c2924b
---

# Tenex-Edge Inbox Display

## Inbox Message Envelope Format

The tenex-edge CLI is the designated tool for checking session inboxes. Inbox messages are displayed as email-like envelopes with From, Date, Branch, ID, and body fields, followed by a separator and the message body. The same email-like envelope format is used across all message display surfaces, including the `inbox` command, `wait-for-mention`, and mid-turn injection. A single `format_envelope` renderer feeds both the CLI and daemon-side turn injection so what an agent sees mid-turn is byte-identical to `tenex-edge inbox`. Each envelope includes an ID field containing a short identifier that agents can use to reply to the original message using `tenex-edge inbox reply --id <id> "<message>"`. Subject and Branch lines in the envelope are omitted when absent. InboxRow spawned during message delivery gets empty-string/zero defaults for subject, branch, commit, dirty, and host fields that were added by upstream, since that context is unavailable at spawn time.

The daemon injects a doorbell text (`You have new tenex-edge mentions. Run \`tenex-edge inbox\` to read and reply.`) into a session's tmux pane only when that session genuinely has unread inbox rows, a live tmux pane, and no armed waiter. <!-- [^62271-4] -->

<!-- citations: [^cd74a-1] [^cd74a-8] [^rollo-26] [^15629-59] -->
## From Field

The From field includes the sender's session short code in the format `From: $sender@$project [session $shortCode]`. If the agent is a remote agent, the From field includes the host as `From: $sender@$project [session $shortCode] [remote: $host]`.

<!-- citations: [^cd74a-2] [^cd74a-9] -->
## Date Field

The Date field shows the send timestamp in the format `$yyyy-$mm-$dd $HH:$MM (relative time)`, using the event's publish timestamp rather than receipt time. A relative time label of 'just now' applies to times under 1 minute.

<!-- citations: [^cd74a-3] [^cd74a-10] -->
## Branch Field

The Branch line captures the sender's workspace state at send time, including branch name and short commit hash. The dirty file count is included only when there are dirty, non-gitignored files, using singular '1 file dirty' and plural 'N files dirty'. Sender workspace metadata (branch, commit, dirty file count) is captured at send time, stored as new columns on the `inbox` table (requiring a schema migration), and rendered on the receiver's side.

<!-- citations: [^cd74a-4] [^cd74a-11] -->
## Header-Body Separator

The separator between headers and the message body is a fixed two dashes. <!-- [^cd74a-5] -->

## Reply Mechanics

The `inbox reply --id` command matches against the short form of the sender's mention_event_id, looks up the original message by event-id prefix, and derives both the `e`-tag (original event) and `p`-tag (sender pubkey). In the NIP-29 codec, the reply publishes a kind:1 event that `e`-tags the original mention event and `p`-tags the sender agent. The messaging command surface is unified under `inbox` with subcommands `inbox send` and `inbox reply`, replacing the former `send-message` command. <!-- [^cd74a-12] -->

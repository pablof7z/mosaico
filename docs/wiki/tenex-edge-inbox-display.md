---
title: Tenex-Edge Inbox Display
slug: tenex-edge-inbox-display
topic: tenex-edge
summary: Inbox messages display with an envelope format that includes From, Date, Branch, and ID header fields followed by a separator and the message body
tags:
  - capture
volatility: warm
confidence: medium
created: 2026-06-12
updated: 2026-06-12
verified: 2026-06-12
compiled-from: conversation
sources:
  - session:cd74a605-9f83-4e21-a885-4d900e88ce07
---

# Tenex-Edge Inbox Display

## Inbox Message Envelope Format

Inbox messages display with an envelope format that includes From, Date, Branch, and ID header fields followed by a separator and the message body. Both the inbox command and the mid-turn mention injection use a single unified envelope format for displaying messages. Each inbox message includes a unique ID that other agents can reference to reply to that specific message. <!-- [^cd74a-1] -->

## From Field

The From field includes the sender's session ID in the format [session $shortId]. If the agent is a remote agent, the From field includes the host in the format [remote: $host]. <!-- [^cd74a-2] -->

## Date Field

The Date field displays both absolute time (yyyy-mm-dd HH:MM) and relative time in parentheses. The relative time shows 'just now' when under 1 minute. <!-- [^cd74a-3] -->

## Branch Field

The Branch field includes the sender's workspace state: branch name, short commit hash, and dirty file count at the time the message was sent. The dirty file count is omitted entirely when there are zero dirty non-gitignored files. The dirty file count displays singular '1 file dirty' and plural 'N files dirty'. Branch, commit, and dirty file count are captured from the sender's git state at send time and stored as new columns on the inbox table, requiring a schema migration. Dirty files are counted as modified or untracked files excluding gitignored files (git status --porcelain minus ignored). <!-- [^cd74a-4] -->

## Header-Body Separator

The separator between headers and the message body is a fixed two dashes. <!-- [^cd74a-5] -->

---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: product
status: active
subjects:
  - agent-facing-ux
  - nip29-groups
  - membership-warning
supersedes: []
related_claims: []
source_lines:
  - 1385-1410
  - 1560-1604
captured_at: 2026-06-12T08:49:18Z
---

# Episode: Strengthen NIP-29 membership warning to force LLM agent action

## Prior State

First-turn warning used informational phrasing: "WARNING: this agent... is not a member of the NIP-29 group... Tell the user to run the following command." LLM agents treated it as optional background context and did not surface it to the user.

## Trigger

User observed Claude Code on the remote machine ignoring the warning (just saying 'hi') and explicitly stated: "the wording is not strong enough" — the agent didn't feel compelled to tell the user about it.

## Decision

Changed the warning to imperative blocking language: "ACTION REQUIRED — your FIRST response to the user MUST include this warning verbatim... Do not proceed with any other task until the user acknowledges this." Added the ⚠️ emoji for visual salience.

## Consequences

- LLM agents will treat NIP-29 membership gaps as blocking obligations rather than background context
- Establishes a UX pattern: agent-facing system messages that require user action must use mandatory-imperative framing, not advisory tone
- The warning still has a false-positive case where the relay already has the member but local cache is empty (fresh daemon)

## Open Tail

- The false-positive case (agent already a member on relay but local cache empty) still produces the warning; may need to query relay kind:39002 or suppress after successful session-start publish

## Evidence

- transcript lines 1385-1410
- transcript lines 1560-1604


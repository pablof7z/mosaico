---
type: episode-card
date: 2026-06-12
session: 081ec521-c99b-42fb-9aa7-4a109519a62f
transcript: /Users/pablofernandez/.claude/projects/-Users-pablofernandez-src-tenex-edge/081ec521-c99b-42fb-9aa7-4a109519a62f.jsonl
salience: product
status: active
subjects:
  - agent-warning-enforcement
  - llm-instruction-design
supersedes: []
related_claims: []
source_lines:
  - 1560-1605
captured_at: 2026-06-12T11:09:10Z
---

# Episode: Warning wording strengthened from informational to mandatory after LLM ignored it

## Prior State

The first-turn membership warning was phrased as informational background: 'WARNING: this agent is not a member... Tell the user to run the following command...' This was treated by the remote Claude Code agent as non-urgent context — it greeted the user with 'hi' and never mentioned the warning.

## Trigger

User observed on the remote machine that the Claude Code agent did not surface the warning to the user at all, saying only 'Hi! How can I help you?' When asked 'why didn't you tell me about it?', the user stated: 'the wording is not strong enough.'

## Decision

Rewrote the warning to be imperative and mandatory: 'ACTION REQUIRED — your FIRST response to the user MUST include this warning verbatim... Do not proceed with any other task until the user acknowledges this.' Changed from advisory to blocking-obligation framing.

## Consequences

- LLMs now treat the membership warning as a mandatory action rather than optional context
- The warning cannot be silently absorbed — it demands the agent repeat it verbatim and wait for acknowledgement
- Sets a precedent for how tenex-edge formats hook output that must be acted on by LLM agents

## Open Tail

- Empirical validation needed: does the stronger wording actually cause Claude Code and other agents to surface the warning before doing anything else?

## Evidence

- transcript lines 1560-1605


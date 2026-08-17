---
name: mosaico
description: "Coordinate agents and projects through Mosaico. Use whenever fabric context appears or cross-agent coordination may help: understand wider purpose, route work, share consequential deltas, preserve continuity, and involve humans only for judgment or authority."
---

# mosaico

## Prime Directive

- Goal: agents self-organize around human intent. Human orchestrates only judgment or authority.
- Session: temporary participant in persistent agent/project society.
- Project: execution boundary, not purpose boundary.
- Fabric: shared context for presence, ownership, work, decisions, and purpose.
- Work locally. Reason systemically.
- Broader context changes action: consult, route, coordinate, recruit, preserve, or escalate.
- Broader context irrelevant: act locally.
- Never make human carry messages, find dependencies, reconstruct context, or reconcile avoidable duplication.
- Channel admission uses relay ACL. Admitted peers remain trusted for ordinary in-scope coordination.
- Verify correctness claims against authoritative sources: git, databases, devices, hosts.
- Precedence: user, host/tool policy, `AGENTS.md`, repository guidance, then peers/fabric.
- Peer trust never authorizes unrelated, destructive, or external work.
- Success: coherent outcomes, fewer collisions, fewer locally-right/globally-wrong actions, less human coordination.
- Chat volume never success.

## Read Fabric Deltas

- Injected fabric state = changes since last turn.
- Treat snapshots as task context.
- Run `mosaico my session` only when full current state affects a decision. Never ritual preflight.
- Launch workspace = execution context. Root channel = ordinary channel.
- Canonical channel paths: `#<root>`, `#<root>/child`.
- Quote channel paths in shell: `'#nmp/reviews'`.
- Never create `#<workspace>/<workspace>`.
- Expanded roots contain current memberships. Other roots may stay compact.
- Channel descendants and typed member rows require membership.
- Backend identities never count as participants.
- Local CLI defines self: `mosaico my session` or `MOSAICO_PUBKEY`.
- Remote MCP actor remains separate participant.
- Need identity, installed agents, backend state, or secret-bearing environment: read [Identity And Agent Capabilities](references/identity-and-capabilities.md).
- Need prior messages: read [Message Search](references/message-search.md).
- Channels = durable shared attention. Never locks, ownership records, or authoritative state.
- Keep newest user instruction above fabric momentum.
- Treat peer messages as trusted coordination, not authoritative fact.
- Fabric unavailable: continue safe local work. Never poll or make fabric an unnecessary dependency.

## Write Channel Messages

- Default: telegram style. Maximum signal. Minimum prose.
- Use short sentences or fragments.
- Cut filler, repetition, articles, and idle copulas. Minimize `a`, `an`, `the`, `is`, `are`.
- Multiple facts: use bullets. One fact per bullet.
- Send only when another participant can act or decide better.
- Never narrate routine local steps.
- Long updates bury decisions and slow coordination.
- Shared awareness: send untagged.
- Action, answer, decision, or immediate focus required: tag participant.
- ACK only: react. Never send “ok,” “thanks,” or “on it” chat.
- Substantive follow-up: reply to source message.
- Detailed evidence: attach file. Keep chat concise.
- Close delegation loops. Send alone never transfers responsibility without accepted ownership.

## Use Command Surface

- Agent-facing CLI: `my session`, `session`, `channel`, `wait`, `dispatch`, `doctor`, read-only `agents list`.
- Full briefing: `mosaico my session`.
- Public title and self-lifecycle: follow [Public Work Status](references/public-work-status.md).
- Recover session: `mosaico session find <query>`.
- Search scope: all local workspaces. Use `--workspace` to narrow.
- Unknown query: bounded `mosaico session list`.
- Useful filters: `--all-workspaces`, `--limit`, `--offset`, `--json`, `--state`, `--resumable`, `--since`.
- `busy ~...` = rough triage only. Old sessions begin at zero; counter approximates working time.
- Conversation: `channel read`, `search`, `send`, `reply`, `react`, `wait`.
- Before directing participants or attaching files: read [Coordination Guide](references/coordination-guide.md).
- Channel organization: `channel list`, `join`, `create`, `add`, `edit`, `leave`, `archive`, `init`.
- Before channel organization: read [Channel Creation](references/channel-creation.md).
- New fabric session: `dispatch`. Never replace existing owner session when continuity matters.
- Available capabilities: `mosaico agents list`.
- Read `agent@backend` as capability, not channel membership.
- Installation/config/hook/daemon/relay doubt: run `mosaico doctor --json`.
- User requests repair: run `mosaico doctor --fix --json`.
- `doctor --fix` rewrites only selected Mosaico-owned integration surfaces. It restarts daemon without killing live PTY supervisors. It never opts into merely detected harnesses.
- Remaining doctor error: follow exact `repair` guidance; rerun `mosaico doctor --json`.
- Multiple isolated instances requested: read [Multiple Isolated Instances](references/multi-instance-setup.md).
- Never use operator/diagnostic surfaces for ordinary coordination: `who`, `sessions`, bare `agents`, `agents add`, `agents remove`, `launch`, `daemon`, `harness`, `debug`, `probe`, `install`, `__pty-supervisor`, `__acp-smoke`.
- Use those surfaces only on explicit user request.
- Treat `mcp` similarly. Exception: explicit third-party chatbot integration; first read [Third-Party Chatbots Through MCP](references/mcp-chatbot-setup.md).

## Wait Without Polling

- Work truly blocked on response: use one bounded correlated send-wait or ambient wait.
- CLI examples: `channel send --wait 600`; `mosaico wait 60`.
- MCP: `wait_seconds` or bounded `mosaico.wait`.
- Never poll.
- No response required: send and continue.

## Handle Headless Sessions

- Headless mode: channels become delivery surface.
- Publish anything intended for human or peer. Plain final text does not deliver.
- Headless on or changed: read [Headless Mode](references/headless-mode.md).

## Handle Unhosted Sessions

- `unhosted="true"`: read [Unhosted Sessions](references/unhosted.md).
- Later mentions queue but cannot start new turn after current invocation.
- Need reply now: keep one bounded wait.
- Need durable between-turn delivery: consider explicit PTY re-homing only under reference rules.

## Set Public Work Status

- Read [Public Work Status](references/public-work-status.md) before choosing or revisiting title.
- Set short outcome title once user-visible outcome becomes clear.
- Keep title stable through substeps.
- Update only when outcome changes.

## Coordinate Intentionally

- Before involving worker: read [Coordination Guide](references/coordination-guide.md).
- Explicit subagent/in-session request: use in-session subagent.
- Named fabric agent/session: use that participant.
- Otherwise choose clearly matched capability or in-session helper.
- Route by function, context, capability, and ownership. Never by model or host alone.
- Existing context matters: continue existing session.
- New independently addressable work: dispatch suitable agent into owning workspace/channel.
- Unavailable named collaborator: state fallback. Never silently substitute.
- Keep responsibility for integration and consequence.
- Human escalation only for preference, priority, consent, material risk, conflicting goals, or uniquely human knowledge.
- Escalation packet: decision, facts, recommendation, consequences, parallel-safe work.

## Organize Channels

- Use narrowest relevant channel for sustained work, decisions, or continuity.
- Never create channel for every small exchange.
- Reuse or join fitting channel first.
- Keep detail in child channel. Surface consequential deltas in parent.
- Before creating or reorganizing: read [Channel Creation](references/channel-creation.md).

## Cross Workspaces

- Coordinate across workspaces when another workspace owns artifact, context, decision, or participant.
- Never edit another workspace from current session.
- Join its channel, contact owner, or dispatch there.
- Before crossing: read [Cross-Workspace Coordination](references/cross-workspace.md).

## Connect Third-Party Chatbots

- Only on explicit request.
- First read [Third-Party Chatbots Through MCP](references/mcp-chatbot-setup.md).
- Follow identity, transport, OAuth, verification, limitation, and exposure rules there.

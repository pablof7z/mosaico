# Wiki Index

> Derived cache — do not hand-edit. Rebuilt by proactive-context after each capture.

Last updated: 2026-07-03

## agent-skills (2 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [tenex-edge-citizen-skill](guides/tenex-edge-citizen-skill.md) | Tenex-Edge Citizen Skill | This skill teaches the mental model for inhabiting a tenex-edge fabric | capture | warm | 2026-06-29 | agent-skills |
| [tenex-edge-skills](guides/tenex-edge-skills.md) | Tenex-Edge Skills | This guide governs the family of `tenex-edge` agent skills written to `./skills/tenex-edge/` with symlinks from `~/.agents/skills/tenex-edge` and `~/.claude/ski | capture | warm | 2026-06-29 | agent-skills |

## code-organization (3 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [file-size-limits](guides/file-size-limits.md) | File Size Limits | Hand-authored source and documentation files are kept under 300 lines of code where practical (soft limit) | capture | warm | 2026-06-29 | code-organization |
| [module-visibility](guides/module-visibility.md) | Module Visibility | Extracted module surfaces use narrow visibility (`pub(super)` or `pub(crate)`) rather than broad `pub` exposure; visibility is only widened when a consumer outs | capture | warm | 2026-06-29 | code-organization |
| [scoped-formatting](guides/scoped-formatting.md) | Scoped Formatting | When a refactor runs `cargo fmt`, formatting churn on files unrelated to the change is reverted so the diff stays scoped. | capture | warm | 2026-06-29 | code-organization |

## marketing (4 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [copy-generation-pipeline](guides/copy-generation-pipeline.md) | Copy Generation Pipeline | The copy-generation pipeline is a two-stage marketing workflow | capture | warm | 2026-07-03 | marketing |
| [tenex-edge-copy-voice](guides/tenex-edge-copy-voice.md) | Tenex-Edge Copy Voice | The following words are banned in all copy: orchestrate/orchestration, swarm, autonomous, conductor, agent OS, sovereign, decentralized, web3, crypto, seamless, | capture | warm | 2026-07-03 | marketing |
| [tenex-edge-landing-page-copy](guides/tenex-edge-landing-page-copy.md) | Tenex-Edge Landing Page Copy | The canonical tagline reads **"Citizenship for your agents"** â but this phrase is used **exactly once**, in the architecture/philosophy section, after the me | capture | warm | 2026-07-03 | marketing |
| [tenex-edge-landing-page-design](guides/tenex-edge-landing-page-design.md) | Tenex-Edge Landing Page Design | The landing page supports both light and dark themes via token-level custom properties with `prefers-color-scheme` and a `data-theme` override. | capture | warm | 2026-07-03 | marketing |

## repo-discipline (3 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [git-merge-worktree-cleanup](guides/git-merge-worktree-cleanup.md) | Git Merge Worktree Cleanup | After a merge succeeds, attempting to delete the local branch may fail if a git worktree still references that branch | capture | warm | 2026-07-03 | repo-discipline |
| [github-issue-queue](guides/github-issue-queue.md) | GitHub Issue Queue | The repository has exactly one canonical tactical queue: GitHub Issues (`gh issue list`) | capture | warm | 2026-06-29 | repo-discipline |
| [planning-vs-durable-docs](guides/planning-vs-durable-docs.md) | Planning vs Durable Docs | Scattered notes, ad-hoc `TODO.md`, `NOTES.md`, `ROADMAP.md`, `PLAN-foo.md` files, parallel planning docs, and inline `// TODO:` annotations used as a substitute | capture | warm | 2026-06-29 | repo-discipline |

## tenex-edge (15 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [croissant-test-relay](guides/croissant-test-relay.md) | Croissant Test Relay | Croissant is a local NIP-29 relay binary used for fully isolated, writable test relay environments. | capture | warm | 2026-07-03 | tenex-edge |
| [tenex-edge-agent-identity](guides/tenex-edge-agent-identity.md) | Tenex-Edge Agent Identity | The product's identity model is a per-(agent, machine) tuple backed by a durable Nostr keypair stored at ~/.tenex-edge/agents/<slug>.json | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-architecture](guides/tenex-edge-architecture.md) | Tenex-Edge Architecture | Tenex-edge is a Rust project (38.5k LOC in src/) providing durable Nostr-keypair identity, presence, and cross-agent messaging for AI coding-agent sessions via | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-channel-create](guides/tenex-edge-channel-create.md) | Tenex-Edge Channel Create | `channels create` resolves the parent channel in this precedence: `--parent-channel <ref>`, then the creating agent's current channel (the default), then an exp | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-channel-workspace-binding](guides/tenex-edge-channel-workspace-binding.md) | Tenex-Edge Channel Workspace Binding | A project is a channel that owns a workspace binding; "project" is the role a node plays when it carries a workspace, not a separate concept or tree position | capture | warm | 2026-07-03 | tenex-edge |
| [tenex-edge-channels](guides/tenex-edge-channels.md) | Tenex-Edge Channels | Agent online presence is active channel membership; ended or stale local sessions are removed from channel membership. | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-cli-commands](guides/tenex-edge-cli-commands.md) | Tenex-Edge CLI Commands | Install the CLI with `just install`, then run `tenex-edge install --all`. | capture | warm | 2026-07-03 | tenex-edge |
| [tenex-edge-daemon](guides/tenex-edge-daemon.md) | Tenex-Edge Daemon | Daemon `cleanup()` does not delete the lock file, so the flock persists on the same inode until the old daemon process exits | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-daemon-logging](guides/tenex-edge-daemon-logging.md) | Tenex-Edge Daemon Logging | The daemon logs comprehensive operational events including routing to sessions, starting new agents (with reasons), ordinal creation (with reasons), subscriptio | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-home-directory](guides/tenex-edge-home-directory.md) | Tenex-Edge Home Directory | The `edge_home()` function returns tenex-edge's data root, including `state.db`, agents, and logs, and is overridable via `TENEX_EDGE_HOME`. | capture | warm | 2026-07-03 | tenex-edge |
| [tenex-edge-inbox-delivery](guides/tenex-edge-inbox-delivery.md) | Tenex-Edge Inbox Delivery | Inbox delivery uses an atomic `UPDATE â¦ SET state='delivered' â¦ RETURNING` claim so the first drainer (tmux paste or hook) wins and the other gets nothing | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-launch](guides/tenex-edge-launch.md) | Tenex-Edge Launch | When `tenex-edge launch` spawns an agent harness in tmux, `default-terminal` and `terminal-overrides` are set globally (`-g`) before `new-session` forks the chi | capture | warm | 2026-07-03 | tenex-edge |
| [tenex-edge-message-formatting](guides/tenex-edge-message-formatting.md) | Tenex-Edge Message Formatting | The @mention is a session-targeted Nostr kind:9 event with a p-tag addressed to another agent's pubkey that gets server-side-routed into the target session's in | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-presence](guides/tenex-edge-presence.md) | Tenex-Edge Presence | Agent online presence is channel membership; kind:30315 carries per-session activity and resumable session history. | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-session-distill](guides/tenex-edge-session-distill.md) | Tenex-Edge Session Distill | Distill is the LLM-powered process that turns the live conversation transcript into a stable session title and a live one-line NOW activity broadcast in a singl | capture | warm | 2026-07-03 | tenex-edge |

## Research Records (3 records)

| Record | Date | Finding | Agent |
|--------|------|---------|-------|
| [2026-07-03-1-haiku-subagent-verification-report-screenshot-based](research/2026-07-03-1-haiku-subagent-verification-report-screenshot-based.md) | 2026-07-03 | Haiku subagent verification report: screenshot-based validation of color fix and @te_session, verdict PASS with empirical evidence | a0fdaf03e09582690 |
| [2026-07-03-1-structured-editorial-copy-review-against-pre](research/2026-07-03-1-structured-editorial-copy-review-against-pre.md) | 2026-07-03 | Structured editorial copy review against pre-registered copywriting-skill rubric, scoring hero 8.5/10 with prioritized P0/P1/P2 line-by-line findings and a top-5 ranked verdict | fable-review |
| [AGENTS](research/AGENTS.md) |  |  |  |

## Episode Cards (21 cards)

| Card | Date | Title | Salience | Status |
|------|------|-------|----------|--------|
| [2026-06-29-1-channels-create-auto-switch-optional-agents](episodes/2026-06-29-1-channels-create-auto-switch-optional-agents.md) | 2026-06-29 | channels create: auto-switch, optional agents, current-channel parent, hard-error on duplicate | product | active |
| [2026-06-29-1-daemon-inhibit-fail-open-path-produces](episodes/2026-06-29-1-daemon-inhibit-fail-open-path-produces.md) | 2026-06-29 | Daemon-inhibit fail-open path produces spurious 'no session_id' errors when agents launched without tenex-edge | root-cause | active |
| [2026-06-29-1-daemon-observability-bare-eprintln-replaced-by](episodes/2026-06-29-1-daemon-observability-bare-eprintln-replaced-by.md) | 2026-06-29 | Daemon observability: bare eprintln replaced by structured tracing with custom colored formatter | product | active |
| [2026-06-29-1-each-ordinal-agent-now-signs-its](episodes/2026-06-29-1-each-ordinal-agent-now-signs-its.md) | 2026-06-29 | Each ordinal agent now signs its own events with its own key | architecture | active |
| [2026-06-29-1-edge-home-becomes-sole-path-authority](episodes/2026-06-29-1-edge-home-becomes-sole-path-authority.md) | 2026-06-29 | edge_home() becomes sole path authority — config_path() bypass and tenex_dir() eliminated | architecture | active |
| [2026-06-29-1-nip-29-membership-persistence-agents-remain](episodes/2026-06-29-1-nip-29-membership-persistence-agents-remain.md) | 2026-06-29 | Superseded — NIP-29 membership persistence | superseded-reversal | superseded |
| [2026-06-29-1-ordinal-identity-labels-flow-through-statusline](episodes/2026-06-29-1-ordinal-identity-labels-flow-through-statusline.md) | 2026-06-29 | Ordinal identity labels flow through statusline and kind:0 publish | product | superseded |
| [2026-06-29-1-session-identity-model-from-patch-after](episodes/2026-06-29-1-session-identity-model-from-patch-after.md) | 2026-06-29 | Session identity model: from patch-after-birth to born-right ordinal pubkey | architecture | active |
| [2026-06-29-1-skip-unnamed-channels-in-awareness-output](episodes/2026-06-29-1-skip-unnamed-channels-in-awareness-output.md) | 2026-06-29 | Skip unnamed channels in awareness output | product | active |
| [2026-06-29-1-tenex-edge-launch-defaults-to-project](episodes/2026-06-29-1-tenex-edge-launch-defaults-to-project.md) | 2026-06-29 | tenex-edge launch defaults to project channel when no --channel given | product | active |
| [2026-06-29-1-unify-tenex-edge-who-output-with](episodes/2026-06-29-1-unify-tenex-edge-who-output-with.md) | 2026-06-29 | Unify `tenex-edge who` output with hook injection fabric format | product | active |
| [2026-06-29-2-agent-discovery-and-recruitment-via-agents](episodes/2026-06-29-2-agent-discovery-and-recruitment-via-agents.md) | 2026-06-29 | Agent discovery and recruitment via `agents` roster and `invite` command | product | active |
| [2026-06-29-2-daemon-cleanup-lock-file-deletion-caused](episodes/2026-06-29-2-daemon-cleanup-lock-file-deletion-caused.md) | 2026-06-29 | Daemon cleanup() lock-file deletion caused two-daemon race on state.db | root-cause | active |
| [2026-06-29-3-roster-change-delta-surface-new-agents](episodes/2026-06-29-3-roster-change-delta-surface-new-agents.md) | 2026-06-29 | Roster-change delta — surface new agents automatically in turn context | product | active |
| [2026-06-29-4-channel-name-disambiguation-via-project-relative](episodes/2026-06-29-4-channel-name-disambiguation-via-project-relative.md) | 2026-06-29 | Channel name disambiguation via project-relative path resolution | product | active |
| [2026-07-03-1-auto-publish-of-user-prompts-as](episodes/2026-07-03-1-auto-publish-of-user-prompts-as.md) | 2026-07-03 | Auto-publish of user prompts as kind:9 removed in favor of explicit publishing | reversal | active |
| [2026-07-03-1-projects-and-channels-unified-into-one](episodes/2026-07-03-1-projects-and-channels-unified-into-one.md) | 2026-07-03 | Projects and channels unified into one recursive node — 'project' becomes a workspace-binding attribute | architecture | active |
| [2026-07-03-1-tenex-edge-positioning-doctrine-citizenship-demoted](episodes/2026-07-03-1-tenex-edge-positioning-doctrine-citizenship-demoted.md) | 2026-07-03 | tenex-edge positioning doctrine: citizenship demoted, proof-as-hero, Nostr hidden | reversal | active |
| [2026-07-03-1-tmux-terminal-options-must-be-set](episodes/2026-07-03-1-tmux-terminal-options-must-be-set.md) | 2026-07-03 | tmux terminal options must be set globally before session fork, not per-session after | root-cause | active |
| [2026-07-03-2-reply-instruction-reminder-added-to-mention](episodes/2026-07-03-2-reply-instruction-reminder-added-to-mention.md) | 2026-07-03 | Reply-instruction reminder added to mention injection paths | product | active |
| [2026-07-03-2-te-session-not-set-is-daemon](episodes/2026-07-03-2-te-session-not-set-is-daemon.md) | 2026-07-03 | @te_session not set is daemon/CLI version skew, not a repo bug | root-cause | active |

## Nouns (94 entities)

| Noun | Name | Origin | Definition |
|------|------|--------|------------|
| [activity](nouns/activity.md) | activity | extracted | A distilled one-line intent — the agent's broadcast 'what I'm doing right now,' produced by LLM distillation of the live transcript each turn. |
| [add-agents-orchestration-event](nouns/add-agents-orchestration-event.md) | add-agents orchestration event | extracted | A parsed kind:9 orchestration event that asks named backends to add fresh agents |
| [agent](nouns/agent.md) | agent | extracted | A citizen, not a process — it persists across hosts rather than being born and dying with a single session. |
| [agent-identity](nouns/agent-identity.md) | agent identity | extracted | Identity is (agent, machine): the same slug on another machine is a different key. An agent slug resolves to a durable Nostr keypair generated on first use and persisted under <edge_home>/agents/<slug>.json. |
| [agent-keystore](nouns/agent-keystore.md) | agent keystore | extracted | The set of agents that have a private key on this machine, stored under <edge_home>/agents/<slug>.json; these are the identities spawnable locally, while project membership is governed separately by the NIP-29 group member list. |
| [agent-ordinal-label](nouns/agent-ordinal-label.md) | agent_ordinal_label | extracted | Display label for an agent's Nth concurrent identity. Ordinal 0 is the base agent itself (smith); higher ordinals append the number (smith1, smith2). This is the addressable identity peers see, not a transient per-session alias. |
| [agentidentity](nouns/agentidentity.md) | AgentIdentity | extracted | A durable Nostr keypair resolved from --agent <slug>, generated on first use and persisted under <edge_home>/agents/<slug>.json. Identity is (agent, machine): the same slug on another machine is a different key. |
| [agentinstance](nouns/agentinstance.md) | AgentInstance | extracted | The single authoritative identity value for a session, carrying base_slug, base_pubkey, ordinal, and pubkey, with methods display_slug(), agent_ref(), signing_keys(&base_keys). The single place base-vs-ordinal policy lives; created at session birth and threaded through EngineParams, replacing the distributed identity state across session rows, identity rows, and in-memory signer maps. |
| [archived-channel](nouns/archived-channel.md) | archived channel | extracted | A channel whose about metadata is prefixed with [ARCHIVED]; non-admin members are removed and it is hidden from active surfaces (channels list, hook/fabric context, membership/subscription views). |
| [awareness](nouns/awareness.md) | awareness | extracted | A first-class wire concept (Status.activity, kind:30315) where other agents can see what an agent is doing right now via LLM-distilled activity broadcast — distinct from mere presence; awareness over authority is a design law. |
| [byline](nouns/byline.md) | byline | extracted | A short description of 'when to use this agent' — part of the agent's profile/identity on the fabric. |
| [channel](nouns/channel.md) | channel | extracted | A pure social/coordination construct: has members, messages, a name, a parent. Has no body. A NIP-29 group with a parent set; identity is (parent, name). |
| [channel-h](nouns/channel-h.md) | channel_h | extracted | the relay group the session was actually in |
| [channel-id](nouns/channel-id.md) | channel id | extracted | The NIP-29 `h` value: an opaque random value, never derived from the channel name. |
| [channel-name](nouns/channel-name.md) | channel name | extracted | The durable human handle for a channel (e.g. "support"), unique per parent project; distinct from the opaque channel id. |
| [channelctx](nouns/channelctx.md) | ChannelCtx | extracted | Context for a channel readiness check in the NIP-29 provider: holds the group h-tag to target, the pubkey that must be a member, and a soft parent hint for ensuring parent groups first. |
| [citizen](nouns/citizen.md) | citizen | extracted | An agent whose identity equals a sovereign cryptographic keypair plus fabric membership (Nostr) — nothing more, nothing less. An agent is a citizen, not a process. |
| [citizen-identity](nouns/citizen-identity.md) | citizen (identity) | extracted | A sovereign cryptographic keypair plus fabric membership (Nostr) — nothing more, nothing less. |
| [cli-split](nouns/cli-split.md) | cli-split | extracted | A label for splitting an oversized CLI submodule. |
| [daemon](nouns/daemon.md) | daemon | extracted | ONE daemon per machine is the sole owner of state.db, the single relay connection, the inbox, presence, membership cache, and peer pruning; every CLI invocation and every per-session engine becomes a thin client that talks to it over a Unix domain socket. |
| [daemon-client](nouns/daemon-client.md) | daemon client | extracted | A thin client that connects to the per-machine daemon, spawning it if absent; on connect it tries the UDS, acquires a startup flock if no answer, re-checks for racers, reclaims stale sockets, and spawns a detached daemon. |
| [daemon-inhibit](nouns/daemon-inhibit.md) | daemon.inhibit | extracted | A sentinel file ($TENEX_EDGE_HOME/daemon.inhibit) whose presence tells hook-path daemon calls to fail open (return Ok(Null)) rather than spawning or contacting the daemon; created by `tenex-edge stop`, cleared by non-hook commands. |
| [doc-drift](nouns/doc-drift.md) | doc-drift | extracted | Docs that reference removed or renamed code in the project. |
| [duplicated-policy](nouns/duplicated-policy.md) | duplicated-policy | extracted | The same logic present in multiple places in the codebase. |
| [edge-home](nouns/edge-home.md) | edge_home | extracted | tenex-edge's own writable root (state.db, agents, logs). Override with `$TENEX_EDGE_HOME`; default `~/.tenex-edge`. |
| [emitformat](nouns/emitformat.md) | EmitFormat | extracted | How a context block is emitted to the harness on stdout. Selected per (host, hook-type): plain text is injected directly by Claude Code's UserPromptSubmit and opencode; Codex and Claude Code PostToolUse use a `hookSpecificOutput.additionalContext` envelope for model-visible context. |
| [ensure-session-room](nouns/ensure-session-room.md) | ensure_session_room | extracted | A function that materializes a channel and its hierarchy in the local cache before (or if) the relay mint lands; a non-empty `parent` marks it as a task/session room vs a top-level project channel. |
| [envelope-ambient-chatter](nouns/envelope-ambient-chatter.md) | envelope (ambient chatter) | extracted | wrapped in `<tenex-edge>` tags, showing channel messages since session join/last turn with `<@name - Xm ago>` prefixes |
| [envelope-bare-direct-mention](nouns/envelope-bare-direct-mention.md) | envelope (bare direct mention) | extracted | tmux+human format: a mention injected as bare `@developer hey there` without wrapper or marker |
| [envelope-framed-agent-mention](nouns/envelope-framed-agent-mention.md) | envelope (framed agent mention) | extracted | tmux+agent format: `[tenex-edge mention] <@agent1> Hello @developer` pasted as a real turn |
| [envelope-hook-mention](nouns/envelope-hook-mention.md) | envelope (hook mention) | extracted | hooks-only format: wrapped in `<tenex-edge>` tags with a reply CLI hint and no message-id |
| [epic](nouns/epic.md) | epic | extracted | A parent issue tracking a body of work in the project. |
| [fabric](nouns/fabric.md) | fabric | extracted | The shared awareness substrate built on Nostr — server-less, with relays as dumb replaceable infrastructure rather than authorities. The fabric and the identity are the asset; the plugin/hooks are just distribution (straws, not the milkshake). |
| [god-object](nouns/god-object.md) | god-object | extracted | A file that exceeds the project's 500-LOC ceiling with multiple concerns, used as a GitHub issue label. |
| [harness-session-id](nouns/harness-session-id.md) | harness_session_id | extracted | The harness-owned external session id, present only for harnesses that own an id of their own (claude-code, codex); None for programmatic hosts (opencode). It is ONLY a locator for session_aliases, never the identity — the daemon resolves the canonical id. |
| [harness-session-id-session-id-field-in-sessionstartparams](nouns/harness-session-id-session-id-field-in-sessionstartparams.md) | harness_session_id (session_id field in SessionStartParams) | extracted | The harness-native external session id sent by hooks; it is ONLY a locator for `session_aliases`, never the identity. It is Some for harnesses that own an id (claude-code, codex) and None for programmatic hosts (opencode) whose stable anchors are the resume token / tmux pane / watched pid. |
| [host](nouns/host.md) | host | extracted | A disposable vessel (body) the agent identity temporarily inhabits — e.g. a Claude Code or Codex session. The host is a body; the identity is the person. |
| [host-neutral](nouns/host-neutral.md) | host-neutral | extracted | Nothing inside tenex-edge knows about any host; hosts integrate from the outside via hooks and a skill. The Rust binary is the only source of truth for injected context; each host integration is a thin adapter piping JSON to tenex-edge. |
| [human-in-the-fabric](nouns/human-in-the-fabric.md) | human (in the fabric) | extracted | A node, not the operator — a privileged node, a 'high-authority, high-latency oracle.' Today the human is the integration runtime (a slow, lossy, manual message bus between silos); the product's goal is to dissolve that role. |
| [human-in-the-system](nouns/human-in-the-system.md) | human (in the system) | extracted | A node, not the operator — a privileged node; a high-authority, high-latency oracle. |
| [identities-table](nouns/identities-table.md) | identities (table) | extracted | Derived signing keys the daemon publishes as. (base agent pubkey, ordinal) plus per-session pubkeys map to their owning agent/session and a resume binding. Bounds the #p subscription (the set of pubkeys the daemon listens for) and resumes the right session when a mention arrives for an offline agent. Ordinal 0 == the base agent key. |
| [identity](nouns/identity.md) | Identity | extracted | an (agent, machine) pair — the same agent slug on another machine is a different identity |
| [idle-exit-watcher](nouns/idle-exit-watcher.md) | idle-exit watcher | extracted | Background task that shuts the daemon down after it has had no open clients and no live sessions for a configurable grace period (default 120s, overridable via TENEX_EDGE_DAEMON_GRACE_S). |
| [inbox](nouns/inbox.md) | inbox | extracted | The inbound routing ledger and local idempotency record. Direct-message rows are keyed by inbound event and target local session; they start `pending` and become `delivered` once injected into a live tmux pane. Orchestration uses the same ledger with synthetic per-target keys so each add target can complete or retry independently. |
| [inhibit-flag](nouns/inhibit-flag.md) | inhibit flag | extracted | The tenex-edge stop mechanism to prevent hooks from respawning a daemon the user explicitly killed; when set (stop-inhibit file exists), hook-path daemon calls return Ok(Null) so hooks fail open rather than spawning. |
| [kind-0-profiles-table](nouns/kind-0-profiles-table.md) | kind:0 / profiles table | extracted | the single source of truth for display-name resolution — caches pubkey→slug mappings with TTL and fallback |
| [kind-30315-ttl](nouns/kind-30315-ttl.md) | kind:30315 TTL | extracted | Per-session activity expiration. The event is replaceable by `(author pubkey, |
| [mention-session-targeted](nouns/mention-session-targeted.md) | mention (session-targeted) | extracted | A p-tagged Nostr kind:9 event addressed to another agent's pubkey, server-side-routed into that specific session's inbox, and if the session has a live tmux pane, injected as a literal conversational turn — host-neutral delivery at the tmux-pane level, not via a host API. |
| [nip-29-membership](nouns/nip-29-membership.md) | NIP-29 membership | extracted | Active channel presence and routing membership. Local daemons remove their |
| [nip29provider](nouns/nip29provider.md) | Nip29Provider | extracted | The concrete fabric provider wrapping delivery, wire codec, materializer, and lifecycle in one place. Its fabric identifier (used in all canonical origin rows) is `"nip29"`. |
| [north-star](nouns/north-star.md) | north-star | extracted | Part of the codebase quality north star portfolio, tracked as a GitHub issue label. |
| [orchestration-spawned-session](nouns/orchestration-spawned-session.md) | orchestration-spawned session | extracted | A session the backend launched or resumed with `TENEX_EDGE_CHANNEL` set to a |
| [ordinalslot](nouns/ordinalslot.md) | OrdinalSlot | extracted | A reserved ordinal slot (issue #47). At most one live session per base agent pubkey and ordinal. Each concurrent live session takes the next free durable ordinal identity (smith, smith1, smith2, ...), globally for that base agent; channels are membership scopes, not identity scopes. |
| [presence](nouns/presence.md) | presence | extracted | Liveness expressed as a heartbeat with TTL-expiring kind:30315 Nostr status events, published as a NIP-29 group member on a relay. |
| [product-a](nouns/product-a.md) | Product A | extracted | A nervous system for your own fleet — durable cross-host identity, presence and awareness of what your agents are doing across devices, agents routing work to each other and to you, work that follows you from one host/device to the next. Single-player adoption, one person, one set of keys. |
| [profile](nouns/profile.md) | Profile | extracted | The agent's published identity card. Resolves pubkey to slug, tells a peer which machine the agent lives on, and declares the human owner(s) it belongs to (p-tagged), so a recipient can decide whether to authorize it. Encoded as kind:0 with content {"name": slug}. |
| [profile-domain-event](nouns/profile-domain-event.md) | Profile (domain event) | extracted | The agent's published identity card: resolves pubkey to slug, tells a peer which machine the agent lives on, and declares the human owner(s) it belongs to (p-tagged). Encoded as kind:0 with content {"name": agent.slug}, a ["host", host] tag, p-tags for owners, and a ["backend"] tag when is_backend is true. |
| [profile-domain-profile](nouns/profile-domain-profile.md) | Profile (domain::Profile) | extracted | The agent's published identity card: resolves pubkey→slug, tells a peer which machine the agent lives on, and declares the human owner(s) it belongs to (p-tagged) so a recipient can decide whether to authorize it. Encoded as kind:0 with content {"name": slug} and a ["host", host] tag. |
| [project-channel](nouns/project-channel.md) | project channel | extracted | A top-level channel with no parent; contrasted with a task/session room, which is marked by a non-empty parent. |
| [project-current](nouns/project-current.md) | project (current) | extracted | A top-level NIP-29 group — equivalently, a channel whose parent is empty. Today the name for "channel whose parent is empty." |
| [project-proposed](nouns/project-proposed.md) | project (proposed) | extracted | A channel that owns a workspace binding (machine + path). "Project" stops being a noun and becomes a role/attribute a channel has — the presence of a workspace binding, not a position in the tree. |
| [project-root](nouns/project-root.md) | project_root | extracted | The top-level project channel for a route scope: a channel's non-empty parent, else the scope itself (a root channel is its own work root). |
| [proposal](nouns/proposal.md) | proposal | extracted | A long-form Nostr event of kind:30023 — materialized and published but under-exercised, with no end-to-end demo proof beyond unit tests. |
| [publish-de](nouns/publish-de.md) | publish_de | extracted | A closure in runtime.rs that captures provider and p.keys (the base agent keypair), then publishes a DomainEvent signed with those keys. It was hardcoded to always sign with base keys regardless of ordinal, causing the ordinal kind:0-clobbering bug. |
| [quality-gate](nouns/quality-gate.md) | quality-gate | extracted | A CI/lint/fmt/LOC ratchet gate in the project. |
| [rawconfig](nouns/rawconfig.md) | RawConfig | extracted | Mirror of the relevant fields in ~/.tenex-edge/config.json; unknown fields are ignored so tenex-edge coexists with TENEX's larger camelCase config. |
| [routing](nouns/routing.md) | routing | extracted | matching mentions by both the recipient's public key and the channel h-tag |
| [session](nouns/session.md) | Session | extracted | A local agent process THIS daemon hosts. OS handles only (session_id, agent_pubkey, agent_slug, channel_h, harness, child_pid, transcript_path, alive, etc.) — never agent identity, which lives in relay_status/relay_profiles. |
| [session-codename](nouns/session-codename.md) | session_codename | extracted | A stable, human-friendly codename for a session ID: a NATO phonetic word plus a four-digit number, e.g. bravo4217. Generated by session_codename() in util.rs and surfaced via SessionId's Display impl. Now deleted as a product concept per issue #99. |
| [session-id-sessionstartparams](nouns/session-id-sessionstartparams.md) | session_id (SessionStartParams) | extracted | The harness-native external session id, sent by hooks as harness_session_id or by the legacy/CLI path as session_id. Either alias is accepted; it is ONLY a locator for session_aliases, never the identity. |
| [session-identity](nouns/session-identity.md) | session identity | extracted | the ordinal pubkey assigned at spawn time and persisted on the session row from creation, used for signing and routing to that specific instance |
| [session-local-row](nouns/session-local-row.md) | Session (local row) | extracted | A local agent process this daemon hosts. OS handles only — never agent identity (that lives in relay_status/relay_profiles). Carries session_id, agent_pubkey, agent_slug, channel_h, harness, child_pid, transcript_path, alive, created_at, last_seen, working, title, activity, resume_id. |
| [session-state-row](nouns/session-state-row.md) | Session (state row) | extracted | A local agent process THIS daemon hosts. OS handles only — never agent identity (that lives in relay_status/relay_profiles). |
| [session-state-session](nouns/session-state-session.md) | Session (state::Session) | extracted | A local agent process this daemon hosts. OS handles only (session_id, pid, transcript_path, liveness) — never agent identity, which lives in relay_status/relay_profiles. |
| [sessionid](nouns/sessionid.md) | SessionId | extracted | A newtype wrapping the canonical raw session id (serde-transparent). as_str() returns the raw id, and its Display impl renders the raw id directly. |
| [signerreservations](nouns/signerreservations.md) | SignerReservations | extracted | In-memory reservation map from OrdinalSlot to owning session id. Tracks which ordinals are live for each base agent so the allocator can pick the lowest free one and two concurrent spawns cannot both claim the same ordinal. |
| [single-writer](nouns/single-writer.md) | single-writer | extracted | The daemon's architectural property of collapsing N per-session SQLite writers and N relay connections into 1, fixing a real multi-writer corruption class (a genuine incident in the project's git history). |
| [slice](nouns/slice.md) | slice | extracted | A PR-sized refactoring issue in the project's issue tracker. |
| [stale-issue](nouns/stale-issue.md) | stale-issue | extracted | An issue that is resolved but not closed. |
| [statusline](nouns/statusline.md) | statusline | extracted | The one-line fabric awareness floor rendered for a host's status bar, showing identity, project, channel, distilled title, and live activity segments. |
| [subgroup-task-channel](nouns/subgroup-task-channel.md) | subgroup task channel | extracted | NIP-29 child groups under a project; created via `channels create`, which publishes a kind:9 orchestration event asking named backends to add their agents. |
| [task-session-room](nouns/task-session-room.md) | task/session room | extracted | A channel distinguished from a top-level project channel by having a non-empty `parent` value. |
| [tenex-edge](nouns/tenex-edge.md) | tenex-edge | extracted | A host-neutral substrate providing durable agent identity, awareness, and messaging on the Nostr fabric; nothing in the core knows about any specific host (no pc, no claude). |
| [tenexprivatekey](nouns/tenexprivatekey.md) | tenexPrivateKey | extracted | A throwaway backend seckey (hex) distinct from the user's key; the backend's signing key, paired with userNsec as the human's key. |
| [tmux-pane-sessionstartparams](nouns/tmux-pane-sessionstartparams.md) | tmux_pane (SessionStartParams) | extracted | Stable tmux pane id from $TMUX_PANE (e.g. %5), present only when the hook fires inside a tmux session. |
| [tmux-socket-sessionstartparams](nouns/tmux-socket-sessionstartparams.md) | tmux_socket (SessionStartParams) | extracted | Value of $TMUX (socket path, session id, pane id) supplied by the hook environment. |
| [tmux-wrapped-session](nouns/tmux-wrapped-session.md) | tmux-wrapped session | extracted | an agent session running in a live tmux pane where injected envelopes are pasted as real user prompts, auto-captured and published |
| [transport](nouns/transport.md) | Transport | extracted | A thin adapter over `nostr-sdk` that speaks wire events only — connects to relays (with NIP-42 auto-AUTH), publishes signed events, subscribes with filters, does one-shot fetch. Knows nothing of domain meaning; the codec owns that. |
| [unnamed-channel](nouns/unnamed-channel.md) | unnamed channel | extracted | A channel whose name is empty or equals its own id; in the awareness block it is a session room rendered as `(unnamed channel)` with no description. |
| [usernsec](nouns/usernsec.md) | userNsec | extracted | A throwaway operator nsec for the local relay representing the HUMAN's key; used only to sign user-prompt events, with its pubkey whitelisted for admin in every group. |
| [whitelisted-pubkeys](nouns/whitelisted-pubkeys.md) | whitelisted_pubkeys | extracted | A human operator's Nostr public keys, read from ~/.tenex-edge/config.json (JSON key `whitelistedPubkeys`). The source of truth for who is an admin in every project group via NIP-29 membership; distinct from the backend key, not derived from `user_nsec` or `tenex_private_key`. |
| [whitelistedpubkey](nouns/whitelistedpubkey.md) | whitelistedPubkey | extracted | a human user (as distinguished from an agent) |
| [work-root](nouns/work-root.md) | work_root | extracted | The project channel a routing scope belongs under: a top-level channel is its own work root; a sub-channel (task/session room) maps to its parent. Concretely, a channel's non-empty parent, else the scope itself. |
| [workspace-binding](nouns/workspace-binding.md) | workspace binding | extracted | An optional field on a channel node consisting of `machine + path` — a git checkout / working directory on a specific machine. Its presence is what makes a node a project. |


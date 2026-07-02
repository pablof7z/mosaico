# Wiki Index

> Derived cache — do not hand-edit. Rebuilt by proactive-context after each capture.

Last updated: 2026-07-02

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

## repo-discipline (2 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [github-issue-queue](guides/github-issue-queue.md) | GitHub Issue Queue | The repository has exactly one canonical tactical queue: GitHub Issues (`gh issue list`) | capture | warm | 2026-06-29 | repo-discipline |
| [planning-vs-durable-docs](guides/planning-vs-durable-docs.md) | Planning vs Durable Docs | Scattered notes, ad-hoc `TODO.md`, `NOTES.md`, `ROADMAP.md`, `PLAN-foo.md` files, parallel planning docs, and inline `// TODO:` annotations used as a substitute | capture | warm | 2026-06-29 | repo-discipline |

## tenex-edge (10 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [tenex-edge-agent-identity](guides/tenex-edge-agent-identity.md) | Tenex-Edge Agent Identity | Agent-instance identity is modeled as one first-class object carried from session birth through all downstream consumers, so that no callsite recomputes which p | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-architecture](guides/tenex-edge-architecture.md) | Tenex-Edge Architecture | The core implementation contract consists of pure domain types, a NIP-29 wire codec, a provider/materializer, a single-writer daemon, and a SQLite read model. | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-channel-create](guides/tenex-edge-channel-create.md) | Tenex-Edge Channel Create | `channels create` resolves the parent channel in this precedence: `--parent-channel <ref>`, then the creating agent's current channel (the default), then an exp | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-channels](guides/tenex-edge-channels.md) | Tenex-Edge Channels | Agent online presence is active channel membership; ended or stale local sessions are removed from channel membership. | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-daemon](guides/tenex-edge-daemon.md) | Tenex-Edge Daemon | Daemon `cleanup()` does not delete the lock file, so the flock persists on the same inode until the old daemon process exits | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-daemon-logging](guides/tenex-edge-daemon-logging.md) | Tenex-Edge Daemon Logging | The daemon logs comprehensive operational events including routing to sessions, starting new agents (with reasons), ordinal creation (with reasons), subscriptio | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-home-directory](guides/tenex-edge-home-directory.md) | Tenex-Edge Home Directory | The `edge_home()` function returns tenex-edge's data root â including `state.db`, agents, and logs â and is overridable via the `TENEX_EDGE_HOME` environmen | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-inbox-delivery](guides/tenex-edge-inbox-delivery.md) | Tenex-Edge Inbox Delivery | Inbox delivery uses an atomic `UPDATE â¦ SET state='delivered' â¦ RETURNING` claim so the first drainer (tmux paste or hook) wins and the other gets nothing | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-message-formatting](guides/tenex-edge-message-formatting.md) | Tenex-Edge Message Formatting | When the sender is a whitelisted pubkey (human) and the agent is in a tmux-wrapped session, a direct mention is pasted as a bare turn: `<@pablo> @developer hey | capture | warm | 2026-06-29 | tenex-edge |
| [tenex-edge-presence](guides/tenex-edge-presence.md) | Tenex-Edge Presence | Agent online presence is channel membership; kind:30315 carries per-session activity and resumable session history. | capture | warm | 2026-06-29 | tenex-edge |

## Research Records (1 record)

| Record | Date | Finding | Agent |
|--------|------|---------|-------|
| [AGENTS](research/AGENTS.md) |  |  |  |

## Episode Cards (15 cards)

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

## Nouns (61 entities)

| Noun | Name | Origin | Definition |
|------|------|--------|------------|
| [add-agents-orchestration-event](nouns/add-agents-orchestration-event.md) | add-agents orchestration event | extracted | A parsed kind:9 orchestration event that asks named backends to add their agents to a subgroup channel. |
| [agent-identity](nouns/agent-identity.md) | agent identity | extracted | Identity is (agent, machine): the same slug on another machine is a different key. An agent slug resolves to a durable Nostr keypair generated on first use and persisted under <edge_home>/agents/<slug>.json. |
| [agent-ordinal-label](nouns/agent-ordinal-label.md) | agent_ordinal_label | extracted | Display label for an agent's Nth concurrent identity. Ordinal 0 is the base agent itself (smith); higher ordinals append the number (smith1, smith2). This is the addressable identity peers see, not a transient per-session alias. |
| [agentidentity](nouns/agentidentity.md) | AgentIdentity | extracted | A durable Nostr keypair resolved from --agent <slug>, generated on first use and persisted under <edge_home>/agents/<slug>.json. Identity is (agent, machine): the same slug on another machine is a different key. |
| [agentinstance](nouns/agentinstance.md) | AgentInstance | extracted | The single authoritative identity value for a session, carrying base_slug, base_pubkey, ordinal, and pubkey, with methods display_slug(), agent_ref(), signing_keys(&base_keys). The single place base-vs-ordinal policy lives; created at session birth and threaded through EngineParams, replacing the distributed identity state across session rows, identity rows, and in-memory signer maps. |
| [channel-h](nouns/channel-h.md) | channel_h | extracted | the relay group the session was actually in |
| [channel-id](nouns/channel-id.md) | channel id | extracted | The NIP-29 `h` value: an opaque random value, never derived from the channel name. |
| [channel-name](nouns/channel-name.md) | channel name | extracted | The durable human handle for a channel (e.g. "support"), unique per parent project; distinct from the opaque channel id. |
| [channelctx](nouns/channelctx.md) | ChannelCtx | extracted | Context for a channel readiness check in the NIP-29 provider: holds the group h-tag to target, the pubkey that must be a member, and a soft parent hint for ensuring parent groups first. |
| [daemon](nouns/daemon.md) | daemon | extracted | ONE daemon per machine is the sole owner of state.db, the single relay connection, the inbox, presence, membership cache, and peer pruning; every CLI invocation and every per-session engine becomes a thin client that talks to it over a Unix domain socket. |
| [daemon-client](nouns/daemon-client.md) | daemon client | extracted | A thin client that connects to the per-machine daemon, spawning it if absent; on connect it tries the UDS, acquires a startup flock if no answer, re-checks for racers, reclaims stale sockets, and spawns a detached daemon. |
| [daemon-inhibit](nouns/daemon-inhibit.md) | daemon.inhibit | extracted | A sentinel file ($TENEX_EDGE_HOME/daemon.inhibit) whose presence tells hook-path daemon calls to fail open (return Ok(Null)) rather than spawning or contacting the daemon; created by `tenex-edge stop`, cleared by non-hook commands. |
| [edge-home](nouns/edge-home.md) | edge_home | extracted | tenex-edge's own writable root (state.db, agents, logs). Override with `$TENEX_EDGE_HOME`; default `~/.tenex-edge`. |
| [emitformat](nouns/emitformat.md) | EmitFormat | extracted | How a context block is emitted to the harness on stdout. Selected per (host, hook-type): plain text is injected directly by Claude Code's UserPromptSubmit and opencode; Codex and Claude Code PostToolUse use a `hookSpecificOutput.additionalContext` envelope for model-visible context. |
| [ensure-session-room](nouns/ensure-session-room.md) | ensure_session_room | extracted | A function that materializes a channel and its hierarchy in the local cache before (or if) the relay mint lands; a non-empty `parent` marks it as a task/session room vs a top-level project channel. |
| [envelope-ambient-chatter](nouns/envelope-ambient-chatter.md) | envelope (ambient chatter) | extracted | wrapped in `<tenex-edge>` tags, showing channel messages since session join/last turn with `<@name - Xm ago>` prefixes |
| [envelope-bare-direct-mention](nouns/envelope-bare-direct-mention.md) | envelope (bare direct mention) | extracted | tmux+human format: a mention injected as bare `@developer hey there` without wrapper or marker |
| [envelope-framed-agent-mention](nouns/envelope-framed-agent-mention.md) | envelope (framed agent mention) | extracted | tmux+agent format: `[tenex-edge mention] <@agent1> Hello @developer` pasted as a real turn |
| [envelope-hook-mention](nouns/envelope-hook-mention.md) | envelope (hook mention) | extracted | hooks-only format: wrapped in `<tenex-edge>` tags with a reply CLI hint and no message-id |
| [harness-session-id](nouns/harness-session-id.md) | harness_session_id | extracted | The harness-owned external session id, present only for harnesses that own an id of their own (claude-code, codex); None for programmatic hosts (opencode). It is ONLY a locator for session_aliases, never the identity — the daemon resolves the canonical id. |
| [harness-session-id-session-id-field-in-sessionstartparams](nouns/harness-session-id-session-id-field-in-sessionstartparams.md) | harness_session_id (session_id field in SessionStartParams) | extracted | The harness-native external session id sent by hooks; it is ONLY a locator for `session_aliases`, never the identity. It is Some for harnesses that own an id (claude-code, codex) and None for programmatic hosts (opencode) whose stable anchors are the resume token / tmux pane / watched pid. |
| [identities-table](nouns/identities-table.md) | identities (table) | extracted | Derived signing keys the daemon publishes as. (base agent pubkey, ordinal) plus per-session pubkeys map to their owning agent/session and a resume binding. Bounds the #p subscription (the set of pubkeys the daemon listens for) and resumes the right session when a mention arrives for an offline agent. Ordinal 0 == the base agent key. |
| [identity](nouns/identity.md) | Identity | extracted | an (agent, machine) pair — the same agent slug on another machine is a different identity |
| [idle-exit-watcher](nouns/idle-exit-watcher.md) | idle-exit watcher | extracted | Background task that shuts the daemon down after it has had no open clients and no live sessions for a configurable grace period (default 120s, overridable via TENEX_EDGE_DAEMON_GRACE_S). |
| [inbox](nouns/inbox.md) | inbox | extracted | The inbound routing ledger and local idempotency record. Direct-message rows are keyed by inbound event and target local session; orchestration rows use synthetic per-target keys so each add target can complete or retry independently. |
| [inhibit-flag](nouns/inhibit-flag.md) | inhibit flag | extracted | The tenex-edge stop mechanism to prevent hooks from respawning a daemon the user explicitly killed; when set (stop-inhibit file exists), hook-path daemon calls return Ok(Null) so hooks fail open rather than spawning. |
| [kind-0-profiles-table](nouns/kind-0-profiles-table.md) | kind:0 / profiles table | extracted | the single source of truth for display-name resolution — caches pubkey→slug mappings with TTL and fallback |
| [kind-30315-ttl](nouns/kind-30315-ttl.md) | kind:30315 TTL | extracted | per-session activity expiration; not the online-presence source |
| [nip-29-membership](nouns/nip-29-membership.md) | NIP-29 membership | extracted | active channel presence and routing membership |
| [nip29provider](nouns/nip29provider.md) | Nip29Provider | extracted | The concrete fabric provider wrapping delivery, wire codec, materializer, and lifecycle in one place. Its fabric identifier (used in all canonical origin rows) is `"nip29"`. |
| [orchestration-spawned-session](nouns/orchestration-spawned-session.md) | orchestration-spawned session | extracted | A session the backend launched with `TENEX_EDGE_CHANNEL` set to add an agent to a task subgroup; it joins that group as-is and does NOT mint a child room. |
| [ordinalslot](nouns/ordinalslot.md) | OrdinalSlot | extracted | A reserved ordinal slot (issue #47). At most one live session per base agent pubkey and ordinal. Each concurrent live session takes the next free durable ordinal identity globally for that base agent; channels are membership scopes, not identity scopes. |
| [profile](nouns/profile.md) | Profile | extracted | The agent's published identity card. Resolves pubkey to slug, tells a peer which machine the agent lives on, and declares the human owner(s) it belongs to (p-tagged), so a recipient can decide whether to authorize it. Encoded as kind:0 with content {"name": slug}. |
| [profile-domain-event](nouns/profile-domain-event.md) | Profile (domain event) | extracted | The agent's published identity card: resolves pubkey to slug, tells a peer which machine the agent lives on, and declares the human owner(s) it belongs to (p-tagged). Encoded as kind:0 with content {"name": agent.slug}, a ["host", host] tag, p-tags for owners, and a ["backend"] tag when is_backend is true. |
| [profile-domain-profile](nouns/profile-domain-profile.md) | Profile (domain::Profile) | extracted | The agent's published identity card: resolves pubkey→slug, tells a peer which machine the agent lives on, and declares the human owner(s) it belongs to (p-tagged) so a recipient can decide whether to authorize it. Encoded as kind:0 with content {"name": slug} and a ["host", host] tag. |
| [project-channel](nouns/project-channel.md) | project channel | extracted | A top-level channel with no parent; contrasted with a task/session room, which is marked by a non-empty parent. |
| [project-root](nouns/project-root.md) | project_root | extracted | The top-level project channel for a route scope: a channel's non-empty parent, else the scope itself (a root channel is its own work root). |
| [publish-de](nouns/publish-de.md) | publish_de | extracted | A closure in runtime.rs that captures provider and p.keys (the base agent keypair), then publishes a DomainEvent signed with those keys. It was hardcoded to always sign with base keys regardless of ordinal, causing the ordinal kind:0-clobbering bug. |
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
| [subgroup-task-channel](nouns/subgroup-task-channel.md) | subgroup task channel | extracted | NIP-29 child groups under a project; created via `channels create`, which publishes a kind:9 orchestration event asking named backends to add their agents. |
| [task-session-room](nouns/task-session-room.md) | task/session room | extracted | A channel distinguished from a top-level project channel by having a non-empty `parent` value. |
| [tenex-dir](nouns/tenex-dir.md) | tenex_dir | extracted | The shared TENEX platform config directory (LLM configs, providers). Override with `$TENEX_DIR`; defaults to the same path as `edge_home()`. Intentionally separate so an isolated `TENEX_EDGE_HOME` daemon still reads real LLM credentials. |
| [tenex-edge](nouns/tenex-edge.md) | tenex-edge | extracted | A host-neutral substrate providing durable agent identity, awareness, and messaging on the Nostr fabric; nothing in the core knows about any specific host (no pc, no claude). |
| [tenexprivatekey](nouns/tenexprivatekey.md) | tenexPrivateKey | extracted | A throwaway backend seckey (hex) distinct from the user's key; the backend's signing key, paired with userNsec as the human's key. |
| [tmux-wrapped-session](nouns/tmux-wrapped-session.md) | tmux-wrapped session | extracted | an agent session running in a live tmux pane where injected envelopes are pasted as real user prompts, auto-captured and published |
| [transport](nouns/transport.md) | Transport | extracted | A thin adapter over `nostr-sdk` that speaks wire events only — connects to relays (with NIP-42 auto-AUTH), publishes signed events, subscribes with filters, does one-shot fetch. Knows nothing of domain meaning; the codec owns that. |
| [unnamed-channel](nouns/unnamed-channel.md) | unnamed channel | extracted | A channel whose name is empty or equals its own id; in the awareness block it is a session room rendered as `(unnamed channel)` with no description. |
| [usernsec](nouns/usernsec.md) | userNsec | extracted | A throwaway operator nsec for the local relay representing the HUMAN's key; used only to sign user-prompt events, with its pubkey whitelisted for admin in every group. |
| [whitelisted-pubkeys](nouns/whitelisted-pubkeys.md) | whitelisted_pubkeys | extracted | A human operator's Nostr public keys, read from ~/.tenex-edge/config.json (JSON key `whitelistedPubkeys`). The source of truth for who is an admin in every project group via NIP-29 membership; distinct from the backend key, not derived from `user_nsec` or `tenex_private_key`. |
| [whitelistedpubkey](nouns/whitelistedpubkey.md) | whitelistedPubkey | extracted | a human user (as distinguished from an agent) |
| [work-root](nouns/work-root.md) | work_root | extracted | The project channel a routing scope belongs under: a top-level channel is its own work root; a sub-channel (task/session room) maps to its parent. Concretely, a channel's non-empty parent, else the scope itself. |

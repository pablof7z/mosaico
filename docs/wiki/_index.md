# Wiki Index

> Derived cache — do not hand-edit. Rebuilt by proactive-context after each capture.

Last updated: 2026-06-16

## code-standards (1 guide)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [code-size-limits](code-size-limits.md) | Code Size Limits | All code files must remain under 500 LOC (hard limit) | capture | warm | 2026-06-10 | code-standards |

## data-synchronization (1 guide)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [tenex-edge-data-synchronization](tenex-edge-data-synchronization.md) | Tenex-Edge Data Synchronization | The Syncthing-synced directory must only synchronize markdown documents and exclude all other file types including git, code, and build artifacts | capture | warm | 2026-06-09 | data-synchronization |

## disk-cleanup (1 guide)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [disk-cleanup](disk-cleanup.md) | Disk Cleanup | Disk cleanup removes only pure build artifacts â compiled output that is regenerable by running the build again | capture | warm | 2026-06-15 | disk-cleanup |

## general (10 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [the-and-keys-increase](the-and-keys-increase.md) | the and keys increase | The `[+]` and `[=]` keys increase the exited sessions hours filter in steps of +1h up to 12h, +6h up to 48h, and +24h beyond that. | capture | warm | 2026-06-14 | general |
| [the-e-key-toggles-the-exited](the-e-key-toggles-the-exited.md) | the e key toggles the exited | The `[e]` key toggles the exited sessions panel on and off, defaulting to a 4-hour window on first enable. | capture | warm | 2026-06-14 | general |
| [the-exited-sessions-section-header-displays](the-exited-sessions-section-header-displays.md) | the exited sessions section header displays | The exited sessions section header displays the active time window (e.g | capture | warm | 2026-06-14 | general |
| [the-tenex-edge-tmux-tui-shows-past](the-tenex-edge-tmux-tui-shows-past.md) | the tenex edge tmux tui shows past | The tenex-edge tmux TUI shows past exited sessions from the past X hours, with the hours filter defaulting to 4 hours. | capture | warm | 2026-06-14 | general |
| [when-the-exited-sessions-panel-is](when-the-exited-sessions-panel-is.md) | when the exited sessions panel is | When the exited sessions panel is visible, the help line shows `[e] hide exited  [-/+] 4h`; when hidden, it shows `[e] show exited`. | capture | warm | 2026-06-14 | general |
| [the-complete-via-rig-distillation-call-must-be](the-complete-via-rig-distillation-call-must-be.md) | the complete via rig distillation call must be | The `complete_via_rig` distillation call runs asynchronously with a 20-second timeout so it does not block the engine loop. | capture | warm | 2026-06-14 | general |
| [the-key-decreases-the-exited](the-key-decreases-the-exited.md) | the key decreases the exited | The `-` key decreases the exited-sessions hours filter in the reverse steps of the increase, with a minimum of 1h. | capture | warm | 2026-06-14 | general |
| [the-last-distill-timestamp-must-only-be](the-last-distill-timestamp-must-only-be.md) | the last distill timestamp must only be | The `last_distill` timestamp is only updated on a successful distillation, not on failure | capture | warm | 2026-06-14 | general |
| [the-sessionstarthookspecificoutputwire-struct-contains-two-f](the-sessionstarthookspecificoutputwire-struct-contains-two-f.md) | the sessionstarthookspecificoutputwire struct contains two f | The SessionStartHookSpecificOutputWire struct contains two fields: suppressOutput and systemMessage. | capture | warm | 2026-06-09 | general |
| [the-tenex-edge-tmux-tui-allows-the](the-tenex-edge-tmux-tui-allows-the.md) | the tenex edge tmux tui allows the | The tenex-edge tmux TUI allows the user to adjust the exited-sessions hours filter using `+`/`-` keys. | capture | warm | 2026-06-14 | general |

## loop-command (1 guide)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [loop-command](loop-command.md) | Loop Command | The /loop command parses input as `[interval] <promptâ¦>` using three priority rules: (1) leading token matching `^\d+[smhd]$`, (2) trailing 'every' clause wit | capture | warm | 2026-06-15 | loop-command |

## session-resumption (1 guide)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [tenex-edge-session-resumption](tenex-edge-session-resumption.md) | Tenex-Edge Session Resumption | Session resume is local-only â a session can only be resumed on the same machine where it ran, not on a remote machine | capture | warm | 2026-06-14 | session-resumption |

## tenex-edge (41 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [opencode-configuration](opencode-configuration.md) | OpenCode Configuration | The @opencode-ai/plugin dependency version must match the opencode binary version (1.16.2) in both ~/.config/opencode/package.json and ~/.opencode/package.json | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge](tenex-edge.md) | Tenex-Edge | tenex-edge is an inversion of TENEX: instead of hosting agents, it grafts a shared coordination fabric onto agents that stay in their native hosts (Claude Code, | capture | warm | 2026-06-07 | tenex-edge |
| [tenex-edge-activity-distillation](tenex-edge-activity-distillation.md) | Tenex-Edge Activity Distillation | Activity distillation is driven by the conversation transcript, not by tool-use events | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-architecture-plan](tenex-edge-architecture-plan.md) | Tenex-Edge Architecture Plan | There is one architecture plan document at `docs/architecture-plan.md`; the brief and the two source research documents have been deleted. | capture | warm | 2026-06-16 | tenex-edge |
| [tenex-edge-awareness](tenex-edge-awareness.md) | Tenex-Edge Awareness | Tenex-edge provides awareness of shared active work, goals, and access to resources | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-channels](tenex-edge-channels.md) | Tenex-Edge Channels | The channel server must be a thin stream-consumer that never independently writes state.db, avoiding re-introduction of multi-writer corruption. | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-configuration](tenex-edge-configuration.md) | Tenex-Edge Configuration | The project slug defaults to the current directory's git repository name (to unify worktrees), or the basename of $PWD if no git repo exists; it can be overridd | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-daemon](tenex-edge-daemon.md) | Tenex-Edge Daemon | The process model is per-session (not a shared daemon) | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-data-persistence](tenex-edge-data-persistence.md) | Tenex-Edge Data Persistence | All data must be read from a unified local interface (e.g., SQLite); how the data is hydrated into that store should be completely irrelevant to the use of that | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-debug-transcript](tenex-edge-debug-transcript.md) | Tenex-Edge Debug Transcript | The `pc debug transcript` command colorizes its output when run on a TTY | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-design-philosophy](tenex-edge-design-philosophy.md) | Tenex-Edge Design Philosophy | The design discussion operates at a higher, design-space levelâwhat the thing is, what shape it should take, what is worth wanting, and where tensions and bet | capture | warm | 2026-06-07 | tenex-edge |
| [tenex-edge-domain-acl](tenex-edge-domain-acl.md) | Tenex-Edge Domain ACL | The ACL feature remains intentionally removed; any stale ACL references are treated as refactor debris to delete rather than restore | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-fabric-architecture](tenex-edge-fabric-architecture.md) | Tenex-Edge Fabric Architecture | A FabricProvider bundles four single-responsibility capabilities: Lifecycle reactor (project spin-up side-effects), Membership source (hydrates and streams the | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-hook-output-rendering](tenex-edge-hook-output-rendering.md) | Tenex-Edge Hook Output Rendering | Hook warnings marked as BLOCKING should be framed as prerequisites to answering, making them harder for the assistant to skip | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-hook-subcommand](tenex-edge-hook-subcommand.md) | Tenex-Edge Hook Subcommand | `tenex-edge hook --host <name> --type <hook-type>` is the sole host-facing entry point for session and turn lifecycle operations | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-host-adapter](tenex-edge-host-adapter.md) | Tenex-Edge Host Adapter | Host adapters must carry no identity logic or fabric logic and must never block the editor on the daemon being healthy (fail open) | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-identity](tenex-edge-identity.md) | Tenex-Edge Identity | Tenex-Edge provides a durable (Nostr) cryptographic identity per agent with session awareness | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-inbox-display](tenex-edge-inbox-display.md) | Tenex-Edge Inbox Display | The tenex-edge CLI is the designated tool for checking session inboxes | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-messaging](tenex-edge-messaging.md) | Tenex-Edge Messaging | In TENEX, all agent-to-agent communication, user-to-agent messages, and project artifacts flow as cryptographically signed Nostr events; each project has its ow | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-nip29-groups](tenex-edge-nip29-groups.md) | Tenex-Edge NIP-29 Groups | The singleton daemon maintains an open subscription to NIP-29 groups it owns at all times, scoped by `#d` for owned project slugs, covering relay-authored group | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-phased-build](tenex-edge-phased-build.md) | Tenex-Edge Phased Build | The fabric-architecture refactor is implemented across 9 sequential phases (0â8) in a git worktree at /Users/pablofernandez/src/tenex-edge-fabric on branch 'f | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-presence](tenex-edge-presence.md) | Tenex-Edge Presence | tenex-edge does not publish 24010/24011 events; received 24011 presence events are ignored, not emitted | capture | warm | 2026-06-10 | tenex-edge |
| [tenex-edge-project-management](tenex-edge-project-management.md) | Tenex-Edge Project Management | tenex-edge project list fetches all kind:39000 events from the relay, caches them in the local project_meta table, and renders them as a left-aligned table. | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-proposals](tenex-edge-proposals.md) | Tenex-Edge Proposals | The proposal (kind:30023) is a tool agents choose to use, not a system-enforced gate; there is no centrally-planned state machine | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-read-model](tenex-edge-read-model.md) | Tenex-Edge Read Model | The read model is the contract; the provider is a write-side materializer | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-relay-configuration](tenex-edge-relay-configuration.md) | Tenex-Edge Relay Configuration | Presence, activity, status, and mention events all use the NIP-29 h tag with the project slug as a namespace filter, replacing the previous T tag | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-relay-ingest](tenex-edge-relay-ingest.md) | Tenex-Edge Relay Ingest | The `handle_incoming` function deduplicates relay events by event ID using a 512-slot ring buffer (`seen_events`) in `DaemonState` to prevent fanout duplication | capture | warm | 2026-06-10 | tenex-edge |
| [tenex-edge-remote-deployment](tenex-edge-remote-deployment.md) | Tenex-Edge Remote Deployment | The tenex-edge project must be cloned on pablo@157.180.102.242 at ~/Work/tenex-edge/ and configured for use with Claude Code | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-session-display](tenex-edge-session-display.md) | Tenex-Edge Session Display | Session IDs displayed in the `tenex tail` command use the hash-based `session_short_code`, matching the display format used by `who` and `send-message` | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-session-management](tenex-edge-session-management.md) | Tenex-Edge Session Management | The MVP (M1) launches a session via `tenex-edge session-start --agent <agent-slug>`, which forks a background process, creates a session ID, and publishes a pre | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-statusline](tenex-edge-statusline.md) | Tenex-Edge Statusline | The statusline is a one-line awareness board representing the floor product (identity + awareness + passive collision signal) in the host terminal | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-tail-design](tenex-edge-tail-design.md) | Tenex-Edge Tail Design | The tail v2 command provides a structured TailEvent stream with 9 variants (Msg, Sync, Turn, Join, Leave, Sess, Status, Proj, Profile), heartbeatâjoin/leave d | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-tail-stream](tenex-edge-tail-stream.md) | Tenex-Edge Tail Stream | The canonical store deduplicates writes on event id, but the tail v2 broadcast emits duplicate messages because one message produces identical tail events for e | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-thread-storage](tenex-edge-thread-storage.md) | Tenex-Edge Thread Storage | The thread dual-write infrastructure (local SQLite read-model of relay conversations via `projects`, `threads`, `messages`, `message_recipients` tables) was rem | capture | warm | 2026-06-10 | tenex-edge |
| [tenex-edge-tmux-adapter](tenex-edge-tmux-adapter.md) | Tenex-Edge TMUX Adapter | A Fable agent is used to plan the TMUX adapter product for tenex-edge | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-transport-codec](tenex-edge-transport-codec.md) | Tenex-Edge Transport Codec | Envelope encoding and decoding is modularized as a codec set providing per-event encode, decode, and subscribe operations, decoupling envelope shapes from busin | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-tui](tenex-edge-tui.md) | Tenex-Edge TUI | The TUI renders immediately from last-known state in SQLite on startup without waiting for relay connection | capture | warm | 2026-06-14 | tenex-edge |
| [tenex-edge-turn-context-injection](tenex-edge-turn-context-injection.md) | Tenex-Edge Turn Context Injection | The turn-start command itself emits the context the agent should see (inbox messages, peer presence/status changes since last update), rather than delegating th | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-user-prompt-submit](tenex-edge-user-prompt-submit.md) | Tenex-Edge User Prompt Submit | The UserPromptSubmit hook creates a kind:1 OP (root event with no e-tag) signed by the userNsec from ~/.tenex/config.json, published to the NIP-29 group via an | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-wait-for-mention](tenex-edge-wait-for-mention.md) | Tenex-Edge Wait-for-Mention | The `wait-for-mention` command polls the SQLite inbox every 500ms, performs the same relay self-fetch as `inbox` on startup to handle the engine warmup race, an | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-off-publishing](tenex-off-publishing.md) | Tenex-Off Publishing | Tenex-off is a direct Nostr client that publishes kind:1 events signed with the owner's nsec straight to relays; it does not call a send-message tool or route t | capture | warm | 2026-06-09 | tenex-edge |

## version-control (1 guide)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [tenex-branch-management](tenex-branch-management.md) | Branch Management | Divergent branches must be resolved through a proper merge that preserves all work from both sides, not via a force-push. | capture | warm | 2026-06-13 | version-control |

## Research Records (1 record)

| Record | Date | Finding | Agent |
|--------|------|---------|-------|
| [2026-06-12-1-review-of-fabric-architecture-branch-verdict](research/2026-06-12-1-review-of-fabric-architecture-branch-verdict.md) | 2026-06-12 | Review of fabric-architecture branch: verdict is refactor is complete, working, and well-tested but no longer cleanly mergeable due to master divergence (~29 conflict hunks) | main |

## Episode Cards (118 cards)

| Card | Date | Title | Salience | Status |
|------|------|-------|----------|--------|
| [2026-06-07-1-product-identity-reframed-from-coordination-tool](episodes/2026-06-07-1-product-identity-reframed-from-coordination-tool.md) | 2026-06-07 | Product identity reframed from coordination tool to agent citizenship protocol | reversal | active |
| [2026-06-07-2-scope-split-into-two-products-customs](episodes/2026-06-07-2-scope-split-into-two-products-customs.md) | 2026-06-07 | Scope split into two products — customs office before open borders | architecture | active |
| [2026-06-07-3-not-greenfield-existing-fabric-makes-tenex](episodes/2026-06-07-3-not-greenfield-existing-fabric-makes-tenex.md) | 2026-06-07 | Not greenfield — existing fabric makes tenex-edge an on-ramp, not a new product | root-cause | active |
| [2026-06-08-1-agent-status-distillation-transcript-first-native](episodes/2026-06-08-1-agent-status-distillation-transcript-first-native.md) | 2026-06-08 | Agent status distillation: transcript-first, native rig, ~/.tenex config | product | superseded |
| [2026-06-08-1-opencode-plugin-version-mismatch-caused-startup](episodes/2026-06-08-1-opencode-plugin-version-mismatch-caused-startup.md) | 2026-06-08 | opencode plugin version-mismatch caused startup hang | root-cause | active |
| [2026-06-08-2-dual-rustls-cryptoprovider-panic-resolved-by](episodes/2026-06-08-2-dual-rustls-cryptoprovider-panic-resolved-by.md) | 2026-06-08 | Dual rustls CryptoProvider panic resolved by installing ring default | root-cause | active |
| [2026-06-09-1-abandon-python-hook-wrapper-harnesses-call](episodes/2026-06-09-1-abandon-python-hook-wrapper-harnesses-call.md) | 2026-06-09 | Abandon Python hook wrapper — harnesses call tenex-edge binary directly | architecture | active |
| [2026-06-09-1-activity-distillation-from-tool-driven-to](episodes/2026-06-09-1-activity-distillation-from-tool-driven-to.md) | 2026-06-09 | Activity distillation: from tool-driven to turn-driven transcript-only model | reversal | active |
| [2026-06-09-1-cli-lifecycle-verbs-removed-hook-becomes](episodes/2026-06-09-1-cli-lifecycle-verbs-removed-hook-becomes.md) | 2026-06-09 | CLI lifecycle verbs removed; hook becomes sole host-facing entry point | architecture | active |
| [2026-06-09-1-codex-sessionstart-hook-must-output-json](episodes/2026-06-09-1-codex-sessionstart-hook-must-output-json.md) | 2026-06-09 | Codex SessionStart hook must output JSON, not plain text | root-cause | active |
| [2026-06-09-1-colorize-pc-debug-transcript-extract-with](episodes/2026-06-09-1-colorize-pc-debug-transcript-extract-with.md) | 2026-06-09 | Colorize pc debug transcript/extract with role-aware ANSI styling | product | active |
| [2026-06-09-1-cqrs-read-model-is-contract-replaces](episodes/2026-06-09-1-cqrs-read-model-is-contract-replaces.md) | 2026-06-09 | CQRS read-model-is-contract replaces provider-through-reads | architecture | active |
| [2026-06-09-1-daemon-actively-owns-nip-29-group](episodes/2026-06-09-1-daemon-actively-owns-nip-29-group.md) | 2026-06-09 | Daemon actively owns NIP-29 group per project | product | active |
| [2026-06-09-1-git-worktree-project-slug-unification-via](episodes/2026-06-09-1-git-worktree-project-slug-unification-via.md) | 2026-06-09 | Git worktree project slug unification via --git-common-dir | root-cause | active |
| [2026-06-09-1-kind-1-mention-vs-activity-disambiguation](episodes/2026-06-09-1-kind-1-mention-vs-activity-disambiguation.md) | 2026-06-09 | Kind:1 Mention vs Activity disambiguation by agent tag | product | active |
| [2026-06-09-1-mcp-channel-adapter-replaces-wait-for](episodes/2026-06-09-1-mcp-channel-adapter-replaces-wait-for.md) | 2026-06-09 | MCP channel adapter replaces wait-for-mention polling hack | architecture | active |
| [2026-06-09-1-package-claude-code-adapter-as-a](episodes/2026-06-09-1-package-claude-code-adapter-as-a.md) | 2026-06-09 | Package Claude Code adapter as a plugin, binary separate | architecture | active |
| [2026-06-09-1-syncthing-stignore-markdown-only-sync-with](episodes/2026-06-09-1-syncthing-stignore-markdown-only-sync-with.md) | 2026-06-09 | Syncthing .stignore: markdown-only sync with correct first-match-wins semantics | root-cause | active |
| [2026-06-09-1-wait-for-mention-blocking-command-as](episodes/2026-06-09-1-wait-for-mention-blocking-command-as.md) | 2026-06-09 | wait-for-mention blocking command as idle-agent wake primitive | product | active |
| [2026-06-09-1-who-display-format-slug-project-slug](episodes/2026-06-09-1-who-display-format-slug-project-slug.md) | 2026-06-09 | who display format: slug@project → slug@hostname | product | active |
| [2026-06-09-2-hook-integration-logic-moved-from-python](episodes/2026-06-09-2-hook-integration-logic-moved-from-python.md) | 2026-06-09 | Hook integration logic moved from Python scripts into Rust binary with data-driven HostDef registry | architecture | active |
| [2026-06-09-2-mention-return-envelope-from-session-threaded](episodes/2026-06-09-2-mention-return-envelope-from-session-threaded.md) | 2026-06-09 | Mention return envelope — from_session threaded end-to-end | product | active |
| [2026-06-09-2-multi-writer-sqlite-is-a-confirmed](episodes/2026-06-09-2-multi-writer-sqlite-is-a-confirmed.md) | 2026-06-09 | Multi-writer SQLite is a confirmed failure mode, not hypothetical | root-cause | superseded |
| [2026-06-09-2-nip-29-groups-were-absent-because](episodes/2026-06-09-2-nip-29-groups-were-absent-because.md) | 2026-06-09 | NIP-29 groups were absent because installed daemon used old relay default | root-cause | active |
| [2026-06-09-2-relay29-authorizes-group-writes-by-event](episodes/2026-06-09-2-relay29-authorizes-group-writes-by-event.md) | 2026-06-09 | Relay29 authorizes group writes by event author, not connection AUTH identity | architecture | active |
| [2026-06-09-2-sessionstart-hooks-cannot-trigger-agent-action](episodes/2026-06-09-2-sessionstart-hooks-cannot-trigger-agent-action.md) | 2026-06-09 | SessionStart hooks cannot trigger agent action — instruction moved to UserPromptSubmit | root-cause | active |
| [2026-06-09-2-single-per-machine-daemon-replaces-per](episodes/2026-06-09-2-single-per-machine-daemon-replaces-per.md) | 2026-06-09 | Single per-machine daemon replaces per-session state.db writers | architecture | active |
| [2026-06-09-2-switch-default-relay-from-relay-tenex](episodes/2026-06-09-2-switch-default-relay-from-relay-tenex.md) | 2026-06-09 | Switch default relay from relay.tenex.chat to nip29.f7z.io | product | active |
| [2026-06-09-2-who-command-shows-project-summaries-instead](episodes/2026-06-09-2-who-command-shows-project-summaries-instead.md) | 2026-06-09 | `who` command shows project summaries instead of per-agent listings for other projects | product | active |
| [2026-06-09-2-who-scope-all-projects-current-project](episodes/2026-06-09-2-who-scope-all-projects-current-project.md) | 2026-06-09 | who scope: all-projects → current-project default with other-projects footer | product | superseded |
| [2026-06-09-3-hash-based-short-session-codes-replace](episodes/2026-06-09-3-hash-based-short-session-codes-replace.md) | 2026-06-09 | Hash-based short session codes replace UUID-prefix truncation | product | active |
| [2026-06-09-3-session-aware-routing-fixes-sibling-session](episodes/2026-06-09-3-session-aware-routing-fixes-sibling-session.md) | 2026-06-09 | Session-aware routing fixes sibling-session mention delivery | root-cause | active |
| [2026-06-09-4-who-output-redesigned-with-rel-cwd](episodes/2026-06-09-4-who-output-redesigned-with-rel-cwd.md) | 2026-06-09 | who output redesigned with rel_cwd and correct remote annotation | product | active |
| [2026-06-12-1-add-tenex-edge-project-add-cli](episodes/2026-06-12-1-add-tenex-edge-project-add-cli.md) | 2026-06-12 | Add `tenex-edge project add` CLI command for NIP-29 group membership | product | active |
| [2026-06-12-1-adopt-fabric-architecture-directly-no-migration](episodes/2026-06-12-1-adopt-fabric-architecture-directly-no-migration.md) | 2026-06-12 | Adopt fabric-architecture directly — no migration or backward compatibility | architecture | active |
| [2026-06-12-1-codec-seam-replaced-by-fabric-provider](episodes/2026-06-12-1-codec-seam-replaced-by-fabric-provider.md) | 2026-06-12 | Codec seam replaced by Fabric Provider architecture | architecture | superseded |
| [2026-06-12-1-inbox-messages-redesigned-from-one-liner](episodes/2026-06-12-1-inbox-messages-redesigned-from-one-liner.md) | 2026-06-12 | Inbox messages redesigned from one-liner to email-like envelope with reply | product | active |
| [2026-06-12-1-integration-mechanism-correction-mcp-server-hooks](episodes/2026-06-12-1-integration-mechanism-correction-mcp-server-hooks.md) | 2026-06-12 | Integration mechanism correction: MCP server → hooks | reversal | active |
| [2026-06-12-1-nip-29-group-membership-management-gap](episodes/2026-06-12-1-nip-29-group-membership-management-gap.md) | 2026-06-12 | NIP-29 group membership management gap — no manual add, no visibility | product | active |
| [2026-06-12-1-provider-seam-closure-must-happen-in](episodes/2026-06-12-1-provider-seam-closure-must-happen-in.md) | 2026-06-12 | Provider seam closure must happen in this task — no deferred wire-shape leaks | architecture | active |
| [2026-06-12-1-remote-agent-display-changed-from-generic](episodes/2026-06-12-1-remote-agent-display-changed-from-generic.md) | 2026-06-12 | Remote agent display changed from generic label to hostname | product | active |
| [2026-06-12-1-secret-scrubbing-layer-before-nostr-event](episodes/2026-06-12-1-secret-scrubbing-layer-before-nostr-event.md) | 2026-06-12 | Secret-scrubbing layer before Nostr event signing | product | active |
| [2026-06-12-1-statusline-as-citizenship-line-not-generic](episodes/2026-06-12-1-statusline-as-citizenship-line-not-generic.md) | 2026-06-12 | Statusline as citizenship line, not generic model bar | product | active |
| [2026-06-12-1-statusline-re-anchored-from-generic-git](episodes/2026-06-12-1-statusline-re-anchored-from-generic-git.md) | 2026-06-12 | Statusline re-anchored from generic git bar to citizenship awareness board | product | superseded |
| [2026-06-12-1-statusline-redesigned-as-citizenship-awareness-line](episodes/2026-06-12-1-statusline-redesigned-as-citizenship-awareness-line.md) | 2026-06-12 | Statusline redesigned as citizenship awareness line | product | superseded |
| [2026-06-12-1-tenex-edge-claude-code-integration-is](episodes/2026-06-12-1-tenex-edge-claude-code-integration-is.md) | 2026-06-12 | tenex-edge Claude Code integration is hooks, not MCP server | architecture | active |
| [2026-06-12-1-who-command-shows-hostname-instead-of](episodes/2026-06-12-1-who-command-shows-hostname-instead-of.md) | 2026-06-12 | who command shows hostname instead of generic (remote) label | product | active |
| [2026-06-12-2-add-tenex-edge-project-add-cli](episodes/2026-06-12-2-add-tenex-edge-project-add-cli.md) | 2026-06-12 | Add `tenex-edge project add` CLI command for manual group membership | product | active |
| [2026-06-12-2-new-tenex-edge-project-add-command](episodes/2026-06-12-2-new-tenex-edge-project-add-command.md) | 2026-06-12 | New `tenex-edge project add` command for NIP-29 group membership | product | active |
| [2026-06-12-2-nip29-f7z-io-added-to-app](episodes/2026-06-12-2-nip29-f7z-io-added-to-app.md) | 2026-06-12 | nip29.f7z.io added to app default relays for fabric reachability | product | superseded |
| [2026-06-12-2-ollama-key-pattern-added-to-scrubber](episodes/2026-06-12-2-ollama-key-pattern-added-to-scrubber.md) | 2026-06-12 | Ollama key pattern added to scrubber | product | active |
| [2026-06-12-2-strengthen-nip-29-membership-warning-to](episodes/2026-06-12-2-strengthen-nip-29-membership-warning-to.md) | 2026-06-12 | Strengthen NIP-29 membership warning to force LLM agent action | product | active |
| [2026-06-12-2-warning-wording-strengthened-from-informational-to](episodes/2026-06-12-2-warning-wording-strengthened-from-informational-to.md) | 2026-06-12 | Warning wording strengthened from informational to mandatory after LLM ignored it | product | active |
| [2026-06-12-3-first-turn-nip-29-membership-warning](episodes/2026-06-12-3-first-turn-nip-29-membership-warning.md) | 2026-06-12 | First-turn NIP-29 membership warning for unauthorized agents | product | active |
| [2026-06-12-3-keyhog-secretscan-rejected-as-unsuitable-for](episodes/2026-06-12-3-keyhog-secretscan-rejected-as-unsuitable-for.md) | 2026-06-12 | Keyhog/secretscan rejected as unsuitable for in-flight redaction | root-cause | active |
| [2026-06-12-4-membership-warning-false-positive-from-stale](episodes/2026-06-12-4-membership-warning-false-positive-from-stale.md) | 2026-06-12 | Membership warning false positive from stale local cache | root-cause | superseded |
| [2026-06-14-1-combined-session-label-distillation-replaces-narrow](episodes/2026-06-14-1-combined-session-label-distillation-replaces-narrow.md) | 2026-06-14 | Combined session-label distillation replaces narrow title-only prompt | product | active |
| [2026-06-14-1-exited-sessions-filter-changed-from-boolean](episodes/2026-06-14-1-exited-sessions-filter-changed-from-boolean.md) | 2026-06-14 | Exited-sessions filter changed from boolean toggle to adjustable time window | product | superseded |
| [2026-06-14-1-resume-spawn-path-for-dead-agent](episodes/2026-06-14-1-resume-spawn-path-for-dead-agent.md) | 2026-06-14 | Resume-spawn path for dead agent sessions | product | superseded |
| [2026-06-14-1-session-resume-any-local-session-resumable](episodes/2026-06-14-1-session-resume-any-local-session-resumable.md) | 2026-06-14 | Session resume: any local session resumable via harness-native token | product | superseded |
| [2026-06-14-1-session-resume-for-any-local-harness](episodes/2026-06-14-1-session-resume-for-any-local-harness.md) | 2026-06-14 | Session resume for any local harness session | product | active |
| [2026-06-14-1-session-resume-for-local-harness-sessions](episodes/2026-06-14-1-session-resume-for-local-harness-sessions.md) | 2026-06-14 | Session Resume for Local Harness Sessions | product | superseded |
| [2026-06-14-1-session-resume-per-harness-resume-commands](episodes/2026-06-14-1-session-resume-per-harness-resume-commands.md) | 2026-06-14 | Session resume: per-harness resume commands and separate resume_id storage | architecture | superseded |
| [2026-06-14-1-spawn-prompt-should-inject-actual-mention](episodes/2026-06-14-1-spawn-prompt-should-inject-actual-mention.md) | 2026-06-14 | Spawn prompt should inject actual mention content, not generic default | product | superseded |
| [2026-06-14-1-tmux-tui-exited-sessions-time-window](episodes/2026-06-14-1-tmux-tui-exited-sessions-time-window.md) | 2026-06-14 | Tmux TUI exited sessions time-window filter | product | superseded |
| [2026-06-14-1-tmux-tui-project-based-session-navigation](episodes/2026-06-14-1-tmux-tui-project-based-session-navigation.md) | 2026-06-14 | Tmux TUI project-based session navigation with tabs, filtering, and search | product | superseded |
| [2026-06-14-1-tmux-tui-redesign-project-tabs-hidden](episodes/2026-06-14-1-tmux-tui-redesign-project-tabs-hidden.md) | 2026-06-14 | Tmux TUI redesign: project tabs, hidden exited sessions, label renames | product | superseded |
| [2026-06-14-1-tmux-tui-redesign-project-tabs-hide](episodes/2026-06-14-1-tmux-tui-redesign-project-tabs-hide.md) | 2026-06-14 | tmux TUI redesign: project tabs, hide exited, concise labels | product | active |
| [2026-06-14-2-claude-code-session-id-env-leak](episodes/2026-06-14-2-claude-code-session-id-env-leak.md) | 2026-06-14 | CLAUDE_CODE_SESSION_ID env leak corrupts spawned session identity | root-cause | superseded |
| [2026-06-14-2-claude-code-session-id-environment-leak](episodes/2026-06-14-2-claude-code-session-id-environment-leak.md) | 2026-06-14 | CLAUDE_CODE_SESSION_ID Environment Leak Corrupting All Spawns | root-cause | active |
| [2026-06-14-2-exited-sessions-hidden-by-default-in](episodes/2026-06-14-2-exited-sessions-hidden-by-default-in.md) | 2026-06-14 | Exited sessions hidden by default in tmux TUI | product | superseded |
| [2026-06-14-2-per-agent-independent-tmux-sessions-replace](episodes/2026-06-14-2-per-agent-independent-tmux-sessions-replace.md) | 2026-06-14 | Per-agent independent tmux sessions replace shared window model | architecture | active |
| [2026-06-14-2-title-distillation-engine-stalls-on-slow](episodes/2026-06-14-2-title-distillation-engine-stalls-on-slow.md) | 2026-06-14 | Title distillation engine stalls on slow/failing API calls | root-cause | superseded |
| [2026-06-14-3-tui-attach-view-session-reaping-race](episodes/2026-06-14-3-tui-attach-view-session-reaping-race.md) | 2026-06-14 | TUI Attach View Session Reaping Race | root-cause | active |
| [2026-06-15-1-delta-gated-posttooluse-sibling-awareness-replaces](episodes/2026-06-15-1-delta-gated-posttooluse-sibling-awareness-replaces.md) | 2026-06-15 | Delta-gated PostToolUse sibling awareness replaces global firehose | product | active |
| [2026-06-15-1-manual-tui-spawns-start-clean-no](episodes/2026-06-15-1-manual-tui-spawns-start-clean-no.md) | 2026-06-15 | Manual TUI spawns start clean — no inbox injection | product | active |
| [2026-06-15-1-narrowed-cleanup-policy-from-aggressive-broad](episodes/2026-06-15-1-narrowed-cleanup-policy-from-aggressive-broad.md) | 2026-06-15 | Narrowed cleanup policy from aggressive broad sweeps to worktree-target-only after near-data-loss | reversal | active |
| [2026-06-15-1-remove-no-tmux-tag-from-tui](episodes/2026-06-15-1-remove-no-tmux-tag-from-tui.md) | 2026-06-15 | Remove [no tmux] tag from TUI session list | product | active |
| [2026-06-15-1-replace-posttooluse-firehose-with-delta-gated](episodes/2026-06-15-1-replace-posttooluse-firehose-with-delta-gated.md) | 2026-06-15 | Replace PostToolUse firehose with delta-gated project-scoped awareness | product | superseded |
| [2026-06-15-1-session-title-distillation-async-immediate-fallback](episodes/2026-06-15-1-session-title-distillation-async-immediate-fallback.md) | 2026-06-15 | Session title distillation: async + immediate fallback + retry-on-failure | root-cause | superseded |
| [2026-06-15-1-tenex-edge-tui-migrates-from-manual](episodes/2026-06-15-1-tenex-edge-tui-migrates-from-manual.md) | 2026-06-15 | tenex-edge TUI migrates from manual crossterm redraw to ratatui | architecture | superseded |
| [2026-06-15-1-tmux-tui-exited-sessions-time-window](episodes/2026-06-15-1-tmux-tui-exited-sessions-time-window.md) | 2026-06-15 | Tmux TUI exited-sessions time-window filter replaces boolean toggle | product | active |
| [2026-06-15-1-tmux-tui-spawn-resolves-project-from](episodes/2026-06-15-1-tmux-tui-spawn-resolves-project-from.md) | 2026-06-15 | tmux TUI spawn resolves project from selected tab, not cwd | root-cause | superseded |
| [2026-06-15-1-tui-rendering-migrated-from-crossterm-full](episodes/2026-06-15-1-tui-rendering-migrated-from-crossterm-full.md) | 2026-06-15 | TUI rendering migrated from crossterm full-clear to ratatui | architecture | active |
| [2026-06-15-1-tui-rendering-migrated-from-manual-crossterm](episodes/2026-06-15-1-tui-rendering-migrated-from-manual-crossterm.md) | 2026-06-15 | TUI rendering migrated from manual crossterm redraw to ratatui | architecture | superseded |
| [2026-06-15-1-tui-spawn-respects-selected-project-tab](episodes/2026-06-15-1-tui-spawn-respects-selected-project-tab.md) | 2026-06-15 | TUI spawn respects selected project tab instead of cwd | product | active |
| [2026-06-15-2-always-visible-session-switcher-sidebar-adopted](episodes/2026-06-15-2-always-visible-session-switcher-sidebar-adopted.md) | 2026-06-15 | Always-visible session-switcher sidebar adopted over popup approach | product | superseded |
| [2026-06-15-2-distillation-engine-async-with-timeout-immediate](episodes/2026-06-15-2-distillation-engine-async-with-timeout-immediate.md) | 2026-06-15 | Distillation engine: async with timeout, immediate prompt-seeded title, retry on failure | architecture | active |
| [2026-06-15-2-popup-quick-switcher-uses-switch-client](episodes/2026-06-15-2-popup-quick-switcher-uses-switch-client.md) | 2026-06-15 | Popup quick-switcher uses switch-client not inline attach | product | active |
| [2026-06-15-2-replace-n-spawn-key-with-enter](episodes/2026-06-15-2-replace-n-spawn-key-with-enter.md) | 2026-06-15 | Replace [n] spawn key with Enter in TUI | product | superseded |
| [2026-06-15-2-session-switching-ux-adopts-phased-approach](episodes/2026-06-15-2-session-switching-ux-adopts-phased-approach.md) | 2026-06-15 | Session-switching UX adopts phased approach: popup prototype then persistent sidebar | product | superseded |
| [2026-06-15-2-session-switching-ux-popup-approach-built](episodes/2026-06-15-2-session-switching-ux-popup-approach-built.md) | 2026-06-15 | Session-switching UX: popup approach built as interim toward persistent sidebar | product | superseded |
| [2026-06-15-2-session-switching-ux-popup-prototype-built](episodes/2026-06-15-2-session-switching-ux-popup-prototype-built.md) | 2026-06-15 | Session switching UX: popup prototype built, sidebar planned | product | superseded |
| [2026-06-15-2-tui-spawn-key-unified-to-enter](episodes/2026-06-15-2-tui-spawn-key-unified-to-enter.md) | 2026-06-15 | TUI spawn key unified to Enter, [no tmux] tag removed | product | active |
| [2026-06-15-3-eliminate-inbox-prompt-injection-on-manual](episodes/2026-06-15-3-eliminate-inbox-prompt-injection-on-manual.md) | 2026-06-15 | Eliminate inbox prompt injection on manual TUI spawns | architecture | superseded |
| [2026-06-15-3-sidebar-fixed-width-pane-per-session](episodes/2026-06-15-3-sidebar-fixed-width-pane-per-session.md) | 2026-06-15 | Sidebar: fixed-width pane-per-session with resize hook | product | active |
| [2026-06-16-1-daemon-cold-start-now-lazy-loads](episodes/2026-06-16-1-daemon-cold-start-now-lazy-loads.md) | 2026-06-16 | Daemon cold-start now lazy-loads relay; accept loop starts immediately | root-cause | active |
| [2026-06-16-1-opencode-plugin-becomes-dumb-pipe-context](episodes/2026-06-16-1-opencode-plugin-becomes-dumb-pipe-context.md) | 2026-06-16 | opencode plugin becomes dumb pipe — context injection unified to Rust hook stdout | architecture | active |
| [2026-06-16-1-session-state-rearchitected-as-single-owner](episodes/2026-06-16-1-session-state-rearchitected-as-single-owner.md) | 2026-06-16 | Session state rearchitected as single-owner aggregate | architecture | superseded |
| [2026-06-16-1-session-state-rebuilt-as-single-owner](episodes/2026-06-16-1-session-state-rebuilt-as-single-owner.md) | 2026-06-16 | Session state rebuilt as single-owner aggregate with daemon-minted identity | architecture | superseded |
| [2026-06-16-1-session-state-redesign-single-owner-aggregate](episodes/2026-06-16-1-session-state-redesign-single-owner-aggregate.md) | 2026-06-16 | Session state redesign: single-owner aggregate with minted identity | architecture | active |
| [2026-06-16-1-session-state-single-owner-aggregate-replaces](episodes/2026-06-16-1-session-state-single-owner-aggregate-replaces.md) | 2026-06-16 | Session state: single-owner aggregate replaces scattered multi-store writes | architecture | superseded |
| [2026-06-16-1-session-title-one-turn-behind-seed](episodes/2026-06-16-1-session-title-one-turn-behind-seed.md) | 2026-06-16 | Session title one-turn-behind seed fix | root-cause | superseded |
| [2026-06-16-1-three-session-state-bugs-diagnosed-and](episodes/2026-06-16-1-three-session-state-bugs-diagnosed-and.md) | 2026-06-16 | Three session-state bugs diagnosed and patched (interim) | root-cause | superseded |
| [2026-06-16-1-title-one-turn-behind-seed-must](episodes/2026-06-16-1-title-one-turn-behind-seed-must.md) | 2026-06-16 | Title one-turn-behind: seed must capture prompt verbatim, not read transcript | root-cause | active |
| [2026-06-16-1-tmux-attach-falls-back-to-resume](episodes/2026-06-16-1-tmux-attach-falls-back-to-resume.md) | 2026-06-16 | tmux attach falls back to resume on stale pane | product | active |
| [2026-06-16-2-distill-never-fires-turn-first-30s](episodes/2026-06-16-2-distill-never-fires-turn-first-30s.md) | 2026-06-16 | Distill never fires: turn_first=30s exceeds most turn durations | root-cause | active |
| [2026-06-16-2-distill-timing-bug-title-never-generated](episodes/2026-06-16-2-distill-timing-bug-title-never-generated.md) | 2026-06-16 | Distill-timing bug — title never generated | root-cause | superseded |
| [2026-06-16-2-full-session-aggregate-architecture-adopted](episodes/2026-06-16-2-full-session-aggregate-architecture-adopted.md) | 2026-06-16 | Full session aggregate architecture adopted | architecture | superseded |
| [2026-06-16-2-nip-40-expiration-on-heartbeat-replaces](episodes/2026-06-16-2-nip-40-expiration-on-heartbeat-replaces.md) | 2026-06-16 | NIP-40 expiration on heartbeat replaces never-expire design | reversal | superseded |
| [2026-06-16-2-nip-40-expiration-on-status-heartbeat](episodes/2026-06-16-2-nip-40-expiration-on-status-heartbeat.md) | 2026-06-16 | NIP-40 expiration on status heartbeat reverses never-expire policy | reversal | superseded |
| [2026-06-16-2-status-heartbeat-events-get-nip-40](episodes/2026-06-16-2-status-heartbeat-events-get-nip-40.md) | 2026-06-16 | Status heartbeat events get NIP-40 expiration, reversing never-expire policy | reversal | superseded |
| [2026-06-16-2-status-heartbeat-events-now-expire-nip](episodes/2026-06-16-2-status-heartbeat-events-now-expire-nip.md) | 2026-06-16 | Status heartbeat events now expire (NIP-40), reversing never-expire policy | reversal | superseded |
| [2026-06-16-2-status-heartbeat-nip-40-expiration-replaces](episodes/2026-06-16-2-status-heartbeat-nip-40-expiration-replaces.md) | 2026-06-16 | Status heartbeat: NIP-40 expiration replaces never-expire policy | reversal | active |
| [2026-06-16-3-full-session-state-aggregate-architecture-adopted](episodes/2026-06-16-3-full-session-state-aggregate-architecture-adopted.md) | 2026-06-16 | Full session-state aggregate architecture adopted | architecture | superseded |
| [2026-06-16-3-nip-40-expiration-on-status-heartbeat](episodes/2026-06-16-3-nip-40-expiration-on-status-heartbeat.md) | 2026-06-16 | NIP-40 expiration on status heartbeat reverses 'never expire' policy | reversal | superseded |
| [2026-06-16-3-session-state-as-single-owner-aggregate](episodes/2026-06-16-3-session-state-as-single-owner-aggregate.md) | 2026-06-16 | Session state as single-owner aggregate with minted identity | architecture | superseded |
| [2026-06-16-4-nip-40-expiration-on-status-heartbeat](episodes/2026-06-16-4-nip-40-expiration-on-status-heartbeat.md) | 2026-06-16 | NIP-40 expiration on status heartbeat — reverses 'never expire' | reversal | superseded |


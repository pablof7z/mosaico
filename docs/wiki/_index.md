# Wiki Index

> Derived cache — do not hand-edit. Rebuilt by proactive-context after each capture.

Last updated: 2026-06-12

## data-synchronization (1 guide)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [tenex-edge-data-synchronization](tenex-edge-data-synchronization.md) | Tenex-Edge Data Synchronization | The Syncthing-synced directory must only synchronize markdown documents and exclude all other file types including git, code, and build artifacts | capture | warm | 2026-06-09 | data-synchronization |

## general (1 guide)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [the-sessionstarthookspecificoutputwire-struct-contains-two-f](the-sessionstarthookspecificoutputwire-struct-contains-two-f.md) | the sessionstarthookspecificoutputwire struct contains two f | The SessionStartHookSpecificOutputWire struct contains two fields: suppressOutput and systemMessage. | capture | warm | 2026-06-09 | general |

## tenex-edge (24 guides)
## tenex-edge (36 guides)

| Slug | Title | Summary | Tags | Volatility | Verified | Topic |
|------|-------|---------|------|------------|----------|-------|
| [opencode-configuration](opencode-configuration.md) | OpenCode Configuration | The @opencode-ai/plugin dependency version must match the opencode binary version (1.16.2) in both ~/.config/opencode/package.json and ~/.opencode/package.json. | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge](tenex-edge.md) | Tenex-Edge | tenex-edge grafts a shared Nostr-based nervous system onto agents that remain in their native hosts (Claude Code, Codex, OpenCode, mobile apps) rather than host | capture | warm | 2026-06-07 | tenex-edge |
| [tenex-edge-activity-distillation](tenex-edge-activity-distillation.md) | Tenex-Edge Activity Distillation | Activity distillation is LLM-based, with an optional heuristic gate to bound cost | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-awareness](tenex-edge-awareness.md) | Tenex-Edge Awareness | The awareness board's state model lives behind a transport interface, so that switching from local storage to network sync is a transport swap rather than a rew | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-channels](tenex-edge-channels.md) | Tenex-Edge Channels | The channel server must be a thin stream-consumer that never independently writes state.db, avoiding re-introduction of multi-writer corruption. | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-configuration](tenex-edge-configuration.md) | Tenex-Edge Configuration | The project slug defaults to the current directory's git repository name (to unify worktrees), or the basename of $PWD if no git repo exists; it can be overridd | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-daemon](tenex-edge-daemon.md) | Tenex-Edge Daemon | tenex-edge uses a single machine-daemon that solely owns state.db, with all CLI calls and session engines acting as thin IPC clients over a Unix domain socket | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-data-persistence](tenex-edge-data-persistence.md) | Tenex-Edge Data Persistence | Local state is stored in SQLite | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-debug-transcript](tenex-edge-debug-transcript.md) | Tenex-Edge Debug Transcript | The `pc debug transcript` command colorizes its output when run on a TTY | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-design-philosophy](tenex-edge-design-philosophy.md) | Tenex-Edge Design Philosophy | The design discussion operates at a higher, design-space level—what the thing is, what shape it should take, what is worth wanting, and where tensions and bets | capture | warm | 2026-06-07 | tenex-edge |
| [tenex-edge-domain-acl](tenex-edge-domain-acl.md) | Tenex-Edge Domain ACL | The domain has two verb planes plus one ACL: Project-State (open_project, roster, presence, status, project_meta, list_projects) and Communications (send, inbox | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-fabric-architecture](tenex-edge-fabric-architecture.md) | Tenex-Edge Fabric Architecture | A FabricProvider bundles four single-responsibility capabilities: Lifecycle reactor (project spin-up side-effects), Membership source (hydrates and streams the | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-hook-subcommand](tenex-edge-hook-subcommand.md) | Tenex-Edge Hook Subcommand | The `hook` subcommand is the only host-facing entry point for harness integrations, dispatching to the same inner functions as the standalone verbs while adding | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-host-adapter](tenex-edge-host-adapter.md) | Tenex-Edge Host Adapter | Host adapters must carry no identity logic or fabric logic and must never block the editor on the daemon being healthy (fail open) | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-identity](tenex-edge-identity.md) | Tenex-Edge Identity | Agent identity is a sovereign keypair, durable per-agent, anchored to a person | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-inbox-display](tenex-edge-inbox-display.md) | Tenex-Edge Inbox Display | Inbox messages display with an envelope format that includes From, Date, Branch, and ID header fields followed by a separator and the message body | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-messaging](tenex-edge-messaging.md) | Tenex-Edge Messaging | Sending a message to another agent uses `tenex-edge send-message` accepting either an agent slug via `<agentSlug>@<projectSlug>` or a session ID via `--recipien | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-nip29-groups](tenex-edge-nip29-groups.md) | Tenex-Edge NIP-29 Groups | The singleton daemon maintains an open subscription for NIP-29 groups it owns at all times | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-phased-build](tenex-edge-phased-build.md) | Tenex-Edge Phased Build | Phase 0 through Phase 8 are executed as a hard sequential dependency chain, with each phase gated by a validation ladder and a commit before proceeding. | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-presence](tenex-edge-presence.md) | Tenex-Edge Presence | tenex-edge does not publish 24010/24011 events; received 24011 presence events are ignored, not emitted | capture | warm | 2026-06-10 | tenex-edge |
| [tenex-edge-project-management](tenex-edge-project-management.md) | Tenex-Edge Project Management | tenex-edge project list fetches all kind:39000 events from the relay, caches them in the local project_meta table, and renders them as a left-aligned table. | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-proposals](tenex-edge-proposals.md) | Tenex-Edge Proposals | The proposal (kind:30023) becomes the only human-facing artifact | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-read-model](tenex-edge-read-model.md) | Tenex-Edge Read Model | The read model is the contract; the provider is a write-side materializer | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-relay-configuration](tenex-edge-relay-configuration.md) | Tenex-Edge Relay Configuration | The default relay is wss://nip29.f7z.io (using nip29) | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-relay-ingest](tenex-edge-relay-ingest.md) | Tenex-Edge Relay Ingest | The `handle_incoming` function deduplicates relay events by event ID using a 512-slot ring buffer (`seen_events`) in `DaemonState` to prevent fanout duplication | capture | warm | 2026-06-10 | tenex-edge |
| [tenex-edge-remote-deployment](tenex-edge-remote-deployment.md) | Tenex-Edge Remote Deployment | The remote machine at pablo@157.180.102.242 must be rebuilt/redeployed after local code changes to get the updated binary. | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-session-display](tenex-edge-session-display.md) | Tenex-Edge Session Display | Session display IDs use a hash-based short code derived from the full UUID rather than truncating the UUID prefix. | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-session-management](tenex-edge-session-management.md) | Tenex-Edge Session Management | MVP1 session start is invoked as `tenex-edge session-start --agent <agent-slug>`, which forks a background process and begins publishing a presence heartbeat | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-tail-stream](tenex-edge-tail-stream.md) | Tenex-Edge Tail Stream | The canonical store deduplicates writes on event id, but the tail v2 broadcast emits duplicate messages because one message produces identical tail events for e | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-transport-codec](tenex-edge-transport-codec.md) | Tenex-Edge Transport Codec | Envelope encoding and decoding is modularized as a codec set providing per-event encode, decode, and subscribe operations, decoupling envelope shapes from busin | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-statusline](tenex-edge-statusline.md) | Tenex-Edge Statusline | The statusline is a one-line awareness board representing the floor product (identity + awareness + passive collision signal) in the host terminal | capture | warm | 2026-06-12 | tenex-edge |
| [tenex-edge-tail-design](tenex-edge-tail-design.md) | Tenex-Edge Tail Design | The tail v2 design (docs/tail-v2-design.md) specifies a structured TailEvent stream, heartbeat-to-join/leave collapse, tiers, read-model backfill, and --json ou | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-thread-storage](tenex-edge-thread-storage.md) | Tenex-Edge Thread Storage | The thread dual-write infrastructure (local SQLite read-model of relay conversations via `projects`, `threads`, `messages`, `message_recipients` tables) was rem | capture | warm | 2026-06-10 | tenex-edge |
| [tenex-edge-transport-codec](tenex-edge-transport-codec.md) | Tenex-Edge Transport Codec | Envelope encoding and decoding is modularized as a codec set providing per-event encode, decode, and subscribe operations | capture | warm | 2026-06-08 | tenex-edge |
| [tenex-edge-turn-context-injection](tenex-edge-turn-context-injection.md) | Tenex-Edge Turn Context Injection | The turn-start command itself emits the context the agent should see (inbox messages, peer presence/status changes since last update), rather than delegating th | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-user-prompt-submit](tenex-edge-user-prompt-submit.md) | Tenex-Edge User Prompt Submit | The user-prompt-submit hook creates a kind:1 Nostr note that is a root event (OP) with no e-tag | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-edge-wait-for-mention](tenex-edge-wait-for-mention.md) | Tenex-Edge Wait-for-Mention | The `wait-for-mention` command polls the SQLite inbox every ~500ms until a mention arrives | capture | warm | 2026-06-09 | tenex-edge |
| [tenex-off-publishing](tenex-off-publishing.md) | Tenex-Off Publishing | Tenex-off is a direct Nostr publisher: it signs and publishes kind:1 notes with the human's nsec and routing tags straight to relays, not via a send-message too | capture | warm | 2026-06-09 | tenex-edge |

## Episode Cards (17 cards)

| Card | Date | Title | Salience |
|------|------|-------|----------|
| [2026-06-12-1-add-tenex-edge-project-add-cli](episodes/2026-06-12-1-add-tenex-edge-project-add-cli.md) | 2026-06-12 | Add `tenex-edge project add` CLI command for NIP-29 group membership | product |
| [2026-06-12-1-inbox-messages-redesigned-from-one-liner](episodes/2026-06-12-1-inbox-messages-redesigned-from-one-liner.md) | 2026-06-12 | Inbox messages redesigned from one-liner to email-like envelope with reply | product |
| [2026-06-12-1-integration-mechanism-correction-mcp-server-hooks](episodes/2026-06-12-1-integration-mechanism-correction-mcp-server-hooks.md) | 2026-06-12 | Integration mechanism correction: MCP server → hooks | reversal |
| [2026-06-12-1-nip-29-group-membership-management-gap](episodes/2026-06-12-1-nip-29-group-membership-management-gap.md) | 2026-06-12 | NIP-29 group membership management gap — no manual add, no visibility | product |
| [2026-06-12-1-remote-agent-display-changed-from-generic](episodes/2026-06-12-1-remote-agent-display-changed-from-generic.md) | 2026-06-12 | Remote agent display changed from generic label to hostname | product |
| [2026-06-12-1-secret-scrubbing-layer-before-nostr-event](episodes/2026-06-12-1-secret-scrubbing-layer-before-nostr-event.md) | 2026-06-12 | Secret-scrubbing layer before Nostr event signing | product |
| [2026-06-12-1-statusline-redesigned-as-citizenship-awareness-line](episodes/2026-06-12-1-statusline-redesigned-as-citizenship-awareness-line.md) | 2026-06-12 | Statusline redesigned as citizenship awareness line | product |
| [2026-06-12-1-tenex-edge-claude-code-integration-is](episodes/2026-06-12-1-tenex-edge-claude-code-integration-is.md) | 2026-06-12 | tenex-edge Claude Code integration is hooks, not MCP server | architecture |
| [2026-06-12-1-who-command-shows-hostname-instead-of](episodes/2026-06-12-1-who-command-shows-hostname-instead-of.md) | 2026-06-12 | who command shows hostname instead of generic (remote) label | product |
| [2026-06-12-2-add-tenex-edge-project-add-cli](episodes/2026-06-12-2-add-tenex-edge-project-add-cli.md) | 2026-06-12 | Add `tenex-edge project add` CLI command for manual group membership | product |
| [2026-06-12-2-new-tenex-edge-project-add-command](episodes/2026-06-12-2-new-tenex-edge-project-add-command.md) | 2026-06-12 | New `tenex-edge project add` command for NIP-29 group membership | product |
| [2026-06-12-2-ollama-key-pattern-added-to-scrubber](episodes/2026-06-12-2-ollama-key-pattern-added-to-scrubber.md) | 2026-06-12 | Ollama key pattern added to scrubber | product |
| [2026-06-12-2-strengthen-nip-29-membership-warning-to](episodes/2026-06-12-2-strengthen-nip-29-membership-warning-to.md) | 2026-06-12 | Strengthen NIP-29 membership warning to force LLM agent action | product |
| [2026-06-12-2-warning-wording-strengthened-from-informational-to](episodes/2026-06-12-2-warning-wording-strengthened-from-informational-to.md) | 2026-06-12 | Warning wording strengthened from informational to mandatory after LLM ignored it | product |
| [2026-06-12-3-first-turn-nip-29-membership-warning](episodes/2026-06-12-3-first-turn-nip-29-membership-warning.md) | 2026-06-12 | First-turn NIP-29 membership warning for unauthorized agents | product |
| [2026-06-12-3-keyhog-secretscan-rejected-as-unsuitable-for](episodes/2026-06-12-3-keyhog-secretscan-rejected-as-unsuitable-for.md) | 2026-06-12 | Keyhog/secretscan rejected as unsuitable for in-flight redaction | root-cause |
| [2026-06-12-4-membership-warning-false-positive-from-stale](episodes/2026-06-12-4-membership-warning-false-positive-from-stale.md) | 2026-06-12 | Membership warning false positive from stale local cache | root-cause |

## Episode Cards (1 card)

| Card | Date | Title | Salience | Status |
|------|------|-------|----------|--------|
| [2026-06-12-1-provider-seam-closure-must-happen-in](episodes/2026-06-12-1-provider-seam-closure-must-happen-in.md) | 2026-06-12 | Provider seam closure must happen in this task — no deferred wire-shape leaks | architecture | active |


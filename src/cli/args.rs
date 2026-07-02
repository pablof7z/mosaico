use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "tenex-edge",
    about = "Citizenship for your agents: identity + awareness on the Nostr fabric."
)]
pub struct Cli {
    #[command(subcommand)]
    pub(super) cmd: Cmd,
}

#[derive(Subcommand)]
pub(super) enum Cmd {
    // session-start / session-end / turn-start / turn-check / turn-end are NOT
    // subcommands. They are hook-driven lifecycle steps invoked only through
    // `harness <name> hook --type <…>`, which parses the harness's stdin payload and calls the
    // corresponding private fn (session_start_inner / session_end / turn_start /
    // turn_check / turn_end). There is no host-facing way — or need — to invoke
    // them by hand.
    /// List agents currently visible in the project/channel.
    Who {
        #[arg(long)]
        project: Option<String>,
        /// Show agents across all projects (overrides --project / cwd resolution).
        #[arg(long)]
        all_projects: bool,
        /// Keep a full-screen live view open, refreshing automatically.
        #[arg(long)]
        live: bool,
    },
    /// Write or read NIP-29 project chat.
    Chat {
        #[command(subcommand)]
        action: ChatAction,
    },
    /// Manage NIP-29 project groups (list, set description).
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Manage NIP-29 subgroup task channels under a project (create, join, leave, list, switch).
    Channels {
        #[command(subcommand)]
        action: ChannelsAction,
    },
    /// Manage the local agent keystore: agents that have a private key on THIS
    /// machine under `<edge_home>/agents/<slug>.json`. These are the identities
    /// you can spawn locally; project membership is governed separately by the
    /// codec (e.g. the NIP-29 group's member list), not here.
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// List invitable agents and prior session ids.
    Agents {
        #[command(subcommand)]
        action: Option<AgentsAction>,
    },
    /// Invite an agent or prior session into a channel. Use --agent for a fresh
    /// session, or --session to restore a prior context.
    Invite {
        /// Project-relative channel name/path/id to invite into.
        #[arg(long)]
        channel: String,
        /// `slug` of a local agent, or `slug@backend-label` where `backend-label`
        /// is the remote backend's config.json `backendName`.
        #[arg(long, conflicts_with = "session", required_unless_present = "session")]
        agent: Option<String>,
        /// Prior session id to resume into the channel.
        #[arg(long, conflicts_with = "agent", required_unless_present = "agent")]
        session: Option<String>,
    },
    /// Hook integration and statusline for any supported agent harness.
    Harness {
        #[command(subcommand)]
        action: HarnessAction,
    },
    /// Publish a long-form proposal (kind:30023) from this agent's session.
    Publish {
        /// Proposal title.
        #[arg(long)]
        title: String,
        /// Proposal body (Markdown). Use "-" or omit to read from stdin.
        #[arg(long = "message", value_name = "BODY")]
        message: Option<String>,
        /// Stable addressable identifier (the kind:30023 `d` tag). Reuse the same
        /// value to publish a REVISION that supersedes a prior proposal at the
        /// same address. Omit to mint a fresh id (a new proposal).
        #[arg(long = "d", value_name = "IDENTIFIER")]
        d: Option<String>,
        /// My session id; if omitted, resolved from the current directory.
        #[arg(long)]
        session: Option<String>,
    },
    /// Launch an agent harness in a new tmux session, with tmux chrome hidden.
    Launch {
        /// Agent slug: "claude", "codex", "opencode", or a local custom agent.
        slug: String,
        /// Project slug; defaults to project resolved from current directory.
        #[arg(long)]
        project: Option<String>,
        /// Channel name to scope this agent into; resolved to its opaque id and
        /// created if absent. Omit the value (`--channel` with no argument) to
        /// open an interactive fuzzy picker over all known rooms for the project.
        /// When per-session rooms
        /// are disabled (the default), omitting `--channel` entirely also opens
        /// the picker; with per-session rooms enabled, omitting it mints a fresh
        /// per-session room instead. The daemon's tenexPrivateKey adds the agent
        /// as a member; if the same derived pubkey is already in the group a
        /// fresh session produces a distinct key via a new anchor, acting as a
        /// second personality.
        #[arg(long, num_args(0..=1), default_missing_value = "")]
        channel: Option<String>,
        /// Override the entire launch command (shell-word split). Replaces the command
        /// stored in the agent file. Example: `-c 'ollama launch claude -- --dangerously-skip-permissions'`
        #[arg(short = 'c', long = "command", value_name = "COMMAND")]
        command_str: Option<String>,
        /// Extra args passed after `--`; appended to the launch command.
        /// Example: `tenex-edge launch codex -- --yolo`
        #[arg(last = true, value_name = "ARGS")]
        extra_args: Vec<String>,
    },
    /// Stop the daemon and prevent hooks from restarting it.
    #[command(hide = true)]
    Stop,
    /// Local debugging tools for hook injection and command telemetry.
    #[command(hide = true)]
    Debug {
        #[command(subcommand)]
        action: DebugAction,
    },
    /// Detect local agent harnesses and wire tenex-edge's hook entries into each.
    #[command(hide = true)]
    Install {
        #[arg(long)]
        all: bool,
        #[arg(long, value_name = "HARNESSES")]
        harness: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        status: bool,
        #[arg(long)]
        uninstall: bool,
    },
    /// Start the per-machine daemon in the foreground.
    #[command(name = "daemon", alias = "__daemon", hide = true)]
    Daemon,
}

#[derive(Subcommand)]
pub(super) enum HarnessAction {
    /// Handle a hook event from a supported agent harness.
    /// Reads hook JSON from stdin; emits context to inject into the model (if any).
    /// Usage: `tenex-edge harness hook <name> --type <hook-type>`
    Hook {
        /// Harness name: claude-code, codex, opencode, grok, …
        /// Run with name "help" to list known harnesses.
        harness: String,
        /// Hook type the harness fires: session-start, user-prompt-submit,
        /// post-tool-use, stop, session-end.
        #[arg(long = "type")]
        hook_type: String,
    },
    /// Render the one-line fabric statusline for a host's status bar.
    /// Reads the harness's statusline JSON payload on stdin (for `session_id`),
    /// prints one line, and always exits 0 — fails open when the daemon is down
    /// (and never spawns one).
    Statusline {
        /// Session id; if omitted, taken from the stdin payload.
        #[arg(long)]
        session: Option<String>,
        /// Emit tmux #[style] format strings instead of ANSI codes. Required
        /// when the output is consumed by tmux's status-format (#(...)).
        #[arg(long)]
        tmux: bool,
    },
}

#[derive(Subcommand)]
pub(super) enum ChatAction {
    /// Publish a project chat line. Reads body from arg, --message, or stdin.
    /// Targets the current agent's active channel; use --channel to override.
    Write {
        /// Message body. Positional, or via --message, or piped on stdin.
        #[arg(value_name = "MESSAGE")]
        message: Option<String>,
        #[arg(long = "message", value_name = "MESSAGE")]
        message_flag: Option<String>,
        /// Project-relative channel name/path/id to write to. Required when
        /// this session is joined to more than one channel; inferred only when
        /// exactly one joined channel exists.
        #[arg(long)]
        channel: Option<String>,
        /// Allow publishing a message longer than the default fabric context cap.
        #[arg(long)]
        long_message: bool,
    },
    /// Read project chat history.
    Read {
        /// Read one exact message by event id; returns the full untruncated body.
        #[arg(long = "id")]
        id: Option<String>,
        /// Only show messages after this time (unix timestamp or duration like "1h").
        #[arg(long)]
        since: Option<String>,
        /// Maximum messages to print.
        #[arg(long)]
        limit: Option<u64>,
        /// Skip this many messages after ordering/filtering.
        #[arg(long)]
        offset: Option<u64>,
        /// Page from the newest messages; output remains chronological.
        #[arg(long)]
        tail: bool,
        /// Keep the chat reader open and print new messages as they arrive.
        #[arg(long)]
        live: bool,
        /// Project-relative channel name/path/id to read. Required when this
        /// session is joined to more than one channel; inferred only when exactly
        /// one joined channel exists.
        #[arg(long, alias = "project")]
        channel: Option<String>,
    },
}

#[derive(Subcommand)]
pub(super) enum AgentAction {
    /// List the agents in this machine's local keystore (slug, pubkey, command).
    List,
    /// Add a local agent: mint + persist its keypair if the slug is new. Pass a
    /// harness launch command after `--` to set how it spawns (e.g.
    /// `tenex-edge agent add reviewer -- claude --dangerously-skip-permissions`);
    /// re-running with a new command overwrites it. With no command, spawning
    /// falls back to the built-in defaults for claude/codex/opencode.
    ///
    /// Repeat `--project <p>` to also assign the agent to one or more projects
    /// in the same step (adds its pubkey to each NIP-29 group).
    Add {
        /// Agent slug ([A-Za-z0-9._-]).
        slug: String,
        /// Assign to this project (repeatable). Adds the agent's pubkey to the
        /// project's NIP-29 group.
        #[arg(long = "project", value_name = "PROJECT")]
        projects: Vec<String>,
        /// Set the harness command as a string (shell-word split). Takes priority
        /// over `--` args. Example: `-c 'ollama launch claude -- --dangerously-skip-permissions'`
        #[arg(short = 'c', long = "command", value_name = "COMMAND")]
        command_str: Option<String>,
        /// Harness launch command (everything after `--`). Optional.
        #[arg(last = true, value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Assign an existing local agent to one or more projects: add its pubkey to
    /// each project's NIP-29 group. Repeat `--project <p>` for multiple projects.
    /// Requires your operator key to be a group admin on the relay.
    Assign {
        /// Agent slug (must already exist in the local keystore).
        slug: String,
        /// Project to assign to (repeatable; at least one required).
        #[arg(long = "project", value_name = "PROJECT", required = true)]
        projects: Vec<String>,
    },
    /// Remove a local agent. Its key file is parked at `<slug>.json.removed`
    /// (not deleted) so a mistake is recoverable; the agent stops being spawnable
    /// and stops being auto-trusted on next read.
    Remove {
        /// Agent slug to remove.
        slug: String,
    },
}

#[derive(Subcommand)]
pub(super) enum AgentsAction {
    /// List the agents this backend can spawn locally.
    List,
    /// List prior session ids grouped by channel.
    ListSessions {
        /// Filter to an agent label or pubkey. `agent@backend-label` preserves the
        /// backend label exactly.
        #[arg(long)]
        agent: Option<String>,
        /// Only show sessions updated after this time (unix timestamp or duration like "2d").
        #[arg(long)]
        since: Option<String>,
    },
}

#[derive(Subcommand)]
pub(super) enum ProjectAction {
    /// List all NIP-29 project groups on the relay.
    List,
    /// Initialize the current directory as a tenex-edge project. Registers the
    /// directory's basename as a slug in `~/.tenex-edge/projects.json`. Refuses
    /// if the slug is already mapped to a different path; pass `--force` to
    /// overwrite. No-op if the slug is already mapped to this exact path.
    Init {
        /// Overwrite an existing slug→path mapping that points elsewhere.
        #[arg(long)]
        force: bool,
    },
    /// Set the description for a project's NIP-29 group (publishes kind:9002).
    Edit {
        /// New description text.
        #[arg(long)]
        description: String,
        /// Project slug; defaults to the project resolved from the current directory.
        #[arg(long)]
        project: Option<String>,
    },
}

/// Subgroup task channels under a project (NIP-29 child groups).
#[derive(Subcommand)]
pub(super) enum ChannelsAction {
    /// Create a subgroup task channel and focus it. When run as an agent
    /// the new channel nests under your CURRENT channel by default, and the
    /// running session auto-joins it. If `--agent slug@backend-label` targets
    /// are named, one kind:9 orchestration event asks those backends to add
    /// their agents.
    Create {
        /// Human-readable channel name, e.g. "support". The channel id (NIP-29
        /// `h` value) is an opaque random value, never derived from the name;
        /// the name is the durable human handle. Unique per parent project.
        #[arg(long)]
        name: String,
        /// Durable channel description, published to the relay as the kind:39000
        /// `about`. Required.
        #[arg(long)]
        about: String,
        /// Optional, repeatable `slug@backend-label`, where `backend-label` is
        /// the target backend's config.json `backendName`. Omit to create an
        /// empty channel.
        #[arg(long = "agent", value_name = "SLUG@BACKEND")]
        agents: Vec<String>,
        /// Parent channel the new channel hangs under. Defaults to the channel
        /// you are currently in; pass a project-relative reference (e.g.
        /// `planning` or `epic999/planning`) to nest it elsewhere in the project.
        #[arg(long = "parent-channel", value_name = "CHANNEL")]
        parent_channel: Option<String>,
    },
    /// List the subgroup task channels under a project.
    List {
        /// Parent project slug. Defaults to the project resolved from the current
        /// directory.
        #[arg(long)]
        project: Option<String>,
    },
    /// Join a channel for passive context and direct-mention delivery.
    Join {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
    },
    /// Stop listening to a passively joined channel.
    Leave {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
    },
    /// Switch the active channel for the current tmux pane to a different NIP-29 subgroup.
    Switch {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
    },
}

#[derive(Subcommand)]
pub(super) enum DebugAction {
    /// Live TUI for hook injections and tenex-edge command invocations.
    HookTail {
        /// Filter panes/events to one or more projects (repeatable).
        #[arg(long = "project")]
        projects: Vec<String>,
        /// Filter panes/events to a session id (or a unique prefix of it).
        #[arg(long)]
        session: Option<String>,
        /// Maximum panes in the grid.
        #[arg(long, default_value = "6")]
        panes: usize,
        /// Refresh interval in milliseconds.
        #[arg(long, default_value = "1000")]
        refresh_ms: u64,
    },
    /// Inspect the status publish outbox.
    Outbox {
        /// Keep printing the outbox state until interrupted.
        #[arg(long)]
        live: bool,
        /// Maximum rows to show.
        #[arg(long, default_value = "50")]
        limit: u64,
        /// Refresh interval in milliseconds when --live is set.
        #[arg(long, default_value = "1000")]
        refresh_ms: u64,
    },
}

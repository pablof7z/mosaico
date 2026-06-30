use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    // `hook --type <…>`, which parses the harness's stdin payload and calls the
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
    /// Stream all fabric activity as structured events, colorized.
    Tail {
        /// Filter to a single project (default: all projects).
        #[arg(long)]
        project: Option<String>,
        /// Filter to a specific agent slug.
        #[arg(long)]
        agent: Option<String>,
        /// Filter to a specific host.
        #[arg(long)]
        host: Option<String>,
        /// Only show events after this time (unix timestamp or duration like "1h").
        #[arg(long)]
        since: Option<String>,
        /// Number of backfill events from history (default 20; 0 = live only).
        #[arg(long)]
        backfill: Option<u64>,
        /// Show only these categories (comma-separated: msg,sync,turn,stat,join,leave,sess,proj,profile).
        #[arg(long)]
        only: Option<String>,
        /// Hide these categories (comma-separated).
        #[arg(long)]
        exclude: Option<String>,
        /// Also show normally-hidden categories (e.g. profile).
        #[arg(long)]
        include: Option<String>,
        /// Show everything including noise (profile, heartbeats).
        #[arg(long, short = 'v')]
        all: bool,
        /// Compact mode: minimal output.
        #[arg(long, short = 'q')]
        compact: bool,
        /// Use relative timestamps ("12s ago") instead of wall-clock.
        #[arg(long)]
        relative: bool,
        /// Disable Unicode glyphs, use ASCII fallbacks.
        #[arg(long)]
        no_emoji: bool,
        /// Disable ANSI colors.
        #[arg(long)]
        no_color: bool,
        /// Output raw NDJSON instead of human-readable lines.
        #[arg(long)]
        json: bool,
        /// Stop after history dump (do not follow live events).
        #[arg(long)]
        no_follow: bool,
        /// Full-screen live TUI dashboard (follow-up feature, not yet implemented).
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
    /// List the agents you can invite into a channel — the local keystore, with
    /// each agent's "when to use" byline. Pull one in with `tenex-edge invite
    /// <slug>`, which spawns a fresh session for it in your current channel.
    Agents,
    /// Invite an agent into your CURRENT channel by spawning a fresh session for
    /// it (the explicit alternative to @-mentioning, which never auto-spawns).
    Invite {
        /// `slug` of a local agent, or `slug@backend` where `backend` is the hex
        /// pubkey/npub of the target backend (defaults to the local backend).
        /// List options with `tenex-edge agents`.
        agent: String,
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
    /// Stop the daemon and prevent hooks from restarting it.
    /// The next non-hook command (who, chat, tail, …) clears the inhibit and
    /// restarts the daemon automatically.
    Stop,
    /// Connectivity check: publish a test note to the configured relays and read it back.
    Doctor,
    /// Local debugging tools for hook injection and command telemetry.
    Debug {
        #[command(subcommand)]
        action: DebugAction,
    },
    /// Handle a hook event from any supported agent harness.
    /// Reads hook JSON from stdin; emits context to inject into the model (if any).
    /// Run `tenex-edge hook --host <name> --type <hook-type>`.
    Hook {
        /// Harness name: "claude-code", "codex", … Run `--host help` to list.
        #[arg(long)]
        host: String,
        /// Hook type the harness uses: "session-start", "user-prompt-submit", etc.
        #[arg(long = "type")]
        hook_type: String,
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
    /// TMUX control-plane commands: status, inject pending messages, spawn agent, attach.
    /// With no subcommand, opens an interactive TUI.
    Tmux {
        #[command(subcommand)]
        action: Option<TmuxAction>,
        /// Run the bare TUI in popup mode: selecting a session switches the
        /// underlying tmux client and exits (closing the `display-popup`),
        /// instead of attaching inline. Used by the `M-t` quick-switcher.
        #[arg(long, hide = true)]
        popup: bool,
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
    /// Detect local agent harnesses (Claude Code, Codex, opencode) and wire
    /// tenex-edge's hook entries into each. With no flags, opens a picker when
    /// interactive and selects detected harnesses in noninteractive shells.
    Install {
        /// Install into every detected harness (skip the interactive picker).
        #[arg(long)]
        all: bool,
        /// Comma-separated harness ids to install (e.g. `claude-code,codex`).
        /// Skips the picker.
        #[arg(long, value_name = "HARNESSES")]
        harness: Option<String>,
        /// Print exactly what would be written without changing anything.
        #[arg(long)]
        dry_run: bool,
        /// Show detection + install status for every known harness and exit.
        #[arg(long)]
        status: bool,
        /// Remove tenex-edge's hooks from the selected harnesses instead of
        /// installing.
        #[arg(long)]
        uninstall: bool,
    },
    /// Start the per-machine daemon in the foreground.
    /// Logs stream to stdout (colorised) and daemon.log simultaneously.
    /// Normally spawned automatically; run this directly to watch its output interactively.
    /// `__daemon` is kept as a hidden alias so the auto-spawner still works without change.
    #[command(name = "daemon", alias = "__daemon")]
    Daemon,
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
        /// Channel name (or id) to write to; resolved to its opaque id within
        /// the sender's project scope. Defaults to this agent's active channel
        /// (TENEX_EDGE_CHANNEL → TENEX_EDGE_SESSION → cwd).
        #[arg(long)]
        channel: Option<String>,
    },
    /// Read project chat history.
    Read {
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
        /// Channel name (or id) to read; defaults to the current agent session's
        /// active channel.
        #[arg(long, alias = "project")]
        channel: Option<String>,
    },
}

#[derive(Subcommand)]
pub(super) enum TmuxAction {
    /// List registered tmux endpoints with liveness info.
    Status,
    /// Manually inject pending messages into a session's pane (debug).
    Send {
        /// Session id (or prefix) to inject.
        #[arg(long)]
        session: String,
    },
    /// Spawn a new tmux window running the given agent harness.
    Spawn {
        /// Agent slug: "claude", "codex", "opencode", …
        #[arg(long)]
        agent: String,
        /// Project slug; defaults to project resolved from current directory.
        #[arg(long)]
        project: Option<String>,
    },
    /// Exec into the tmux pane registered for a session.
    Attach {
        /// Session id (or prefix).
        #[arg(long)]
        session: String,
    },
    /// Resume a (typically dead) session: replay its harness in a new tmux
    /// window using the captured native resume token, then attach to it.
    Resume {
        /// Session id (or a unique prefix of it) to resume.
        #[arg(long)]
        session: String,
    },
    /// Long-running sidebar process: list project sessions in a narrow pane,
    /// highlight the current session, and let the user switch between them.
    /// Normally started automatically by `ensure_sidebar`; can also be run
    /// manually with `tenex-edge tmux sidebar --session <id>`.
    Sidebar {
        /// The session this sidebar belongs to (highlighted as "current").
        /// If omitted, resolved at runtime from the tmux client session name.
        #[arg(long)]
        session: Option<String>,
        /// Project to filter by. If omitted, derived from the current session's
        /// live row in the daemon data.
        #[arg(long)]
        project: Option<String>,
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
    /// running session auto-joins it. If `--agent slug@backend` targets
    /// are named, one kind:9 orchestration event asks those backends to add
    /// their agents.
    Create {
        /// Human-readable channel name, e.g. "support". The channel id (NIP-29
        /// `h` value) is an opaque random value, never derived from the name;
        /// the name is the durable human handle. Unique per parent project.
        #[arg(long)]
        name: String,
        /// Durable channel description, published to the relay as the kind:39000
        /// `about`. Optional.
        #[arg(long)]
        about: Option<String>,
        /// Optional, repeatable `slug@backend`, where `slug` is the agent identity
        /// (the `~/.tenex-edge/agents/*.json` filename stem, e.g. `developer`,
        /// `alice`) and `backend` is a hex pubkey or npub of the target backend
        /// (the pubkey of its tenexPrivateKey). Omit to create an empty channel.
        #[arg(long = "agent", value_name = "SLUG@BACKEND")]
        agents: Vec<String>,
        /// Parent channel the new channel hangs under. Defaults to the channel
        /// you are currently in; pass a project-relative reference (e.g.
        /// `planning` or `epic999/planning`) to nest it elsewhere in the project.
        #[arg(long = "parent-channel", value_name = "CHANNEL")]
        parent_channel: Option<String>,
        /// Path to a markdown brief; its contents become the kind:9 prose body.
        #[arg(long = "message", value_name = "PATH")]
        message: Option<PathBuf>,
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

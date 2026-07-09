use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli) enum AgentAction {
    /// List the agents in this machine's local keystore (slug, commands).
    List,
    /// Add a local agent: mint + persist its keypair if the slug is new. Pass a
    /// harness launch command after `--` to set its default named command (e.g.
    /// `tenex-edge agent add reviewer -- claude --dangerously-skip-permissions`);
    /// re-running with a new command overwrites that default. With no commands,
    /// interactive launch prompts for one and daemon/TUI spawns use built-in
    /// defaults only for built-in harness slugs.
    ///
    /// Repeat `--project <p>` to document the intended assignment. Per-project
    /// roster scoping is not implemented yet; current roster publish advertises
    /// every local capability to every root project.
    Add {
        /// Agent slug ([A-Za-z0-9._-]).
        slug: String,
        /// Intended project assignment (repeatable). Currently triggers a
        /// roster republish; per-project scoping is not implemented yet.
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
    /// Assign an existing local agent to one or more projects. Per-project
    /// roster scoping is not implemented yet; this republished roster still
    /// advertises every local capability to every root project.
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
pub(in crate::cli) enum AgentsAction {
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

/// `channel add` targets. Exactly one of three shapes: a human member by id
/// (two positionals `<id> <channel>`), a freshly spawned session
/// (`--new-session <role>[@machine] <channel>`), or an existing session pulled in
/// (`--session @codename@host <channel>`). Flag modes take ONE positional (the
/// channel); human mode takes TWO. `--admin` is human-only; `--message` posts a
/// chat mentioning the brought-online session and is valid only in the session
/// modes.
#[derive(Args)]
pub(in crate::cli) struct AddArgs {
    /// Human mode: the member id (hex pubkey, npub, or nip05). Flag modes: the
    /// project-relative channel to add into.
    #[arg(value_name = "ID_OR_CHANNEL")]
    pub(in crate::cli::admin) first: Option<String>,
    /// Human mode only: the project-relative channel (second positional).
    #[arg(value_name = "CHANNEL")]
    pub(in crate::cli::admin) second: Option<String>,
    /// Spawn a fresh session of `ROLE[@machine]` and add it to the channel.
    #[arg(long = "new-session", value_name = "ROLE", conflicts_with_all = ["session", "admin"])]
    pub(in crate::cli::admin) new_session: Option<String>,
    /// Pull an existing session, named `@codename@host`, into the channel.
    #[arg(long, value_name = "CODENAME", conflicts_with_all = ["new_session", "admin"])]
    pub(in crate::cli::admin) session: Option<String>,
    /// Grant admin rather than member. Human target only.
    #[arg(long)]
    pub(in crate::cli::admin) admin: bool,
    /// Also post a chat line into the channel mentioning the brought-online
    /// session. Valid only with `--new-session`/`--session`.
    #[arg(long, value_name = "TEXT")]
    pub(in crate::cli::admin) message: Option<String>,
}

/// Subgroup task channels under a project (NIP-29 child groups).
#[derive(Subcommand)]
pub(in crate::cli) enum ChannelAction {
    /// Add a member to a channel: a human by id, a freshly spawned session
    /// (`--new-session <role>`), or an existing one (`--session @codename@host`).
    Add(AddArgs),
    /// Read channel chat history.
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
        #[arg(long)]
        channel: Option<String>,
        /// Explicit reader session id instead of resolving from the current
        /// PTY/harness process or project scan.
        #[arg(long)]
        session: Option<String>,
    },
    /// Send a chat line to a channel. Reads body from arg, --message, or stdin.
    /// Targets the current agent's active channel; use --channel to override.
    Send {
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
        /// Explicit sender session id instead of resolving from the current
        /// PTY/harness process or project scan.
        #[arg(long)]
        session: Option<String>,
        /// Allow publishing a message longer than the default 600-character cap.
        #[arg(long)]
        long_message: bool,
    },
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
        /// Short, stable channel description (max 80 chars), not status text.
        #[arg(long, value_parser = crate::channel_about::parse_channel_about)]
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
        /// Explicit session id to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// Edit metadata on an existing subgroup task channel.
    Edit {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
        /// New durable channel description.
        #[arg(long, value_parser = crate::channel_about::parse_channel_about)]
        about: String,
        /// Explicit session id to use as the project-relative resolution anchor.
        #[arg(long)]
        session: Option<String>,
    },
    /// List the subgroup task channels under a project, or every top-level
    /// project on the relay with `--roots`.
    List {
        /// Parent project slug. Defaults to the project resolved from the current
        /// directory. Ignored with `--roots`.
        #[arg(long)]
        project: Option<String>,
        /// List every top-level project on the relay instead of one project's
        /// subgroup tree.
        #[arg(long, conflicts_with = "project")]
        roots: bool,
    },
    /// Register the current directory as a tenex-edge project workspace. Maps
    /// the directory's basename as a slug in `~/.tenex-edge/projects.json` so a
    /// non-git directory resolves to a project. Refuses if the slug is already
    /// mapped to a different path; pass `--force` to overwrite. No-op if the
    /// slug already maps to this exact path.
    Init {
        /// Overwrite an existing slug->path mapping that points elsewhere.
        #[arg(long)]
        force: bool,
    },
    /// Join a channel for passive context and direct-mention delivery.
    Join {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
        /// Explicit session id to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// Stop listening to a passively joined channel.
    Leave {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
        /// Explicit session id to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// Mark a channel archived and remove all non-admin members.
    Archive {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
        /// Explicit session id to use as the project-relative resolution anchor.
        #[arg(long)]
        session: Option<String>,
    },
    /// Switch the active channel for the current session to a different NIP-29 subgroup.
    Switch {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
        /// Explicit session id to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
}

#[cfg(test)]
mod tests;

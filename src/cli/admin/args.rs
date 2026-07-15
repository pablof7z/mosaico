use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli) enum AgentAction {
    /// List the agents in this machine's local keystore.
    List,
    /// Add a local agent and select its required harness bundle.
    ///
    /// Repeat `--workspace <p>` to document the intended assignment. Per-workspace
    /// roster scoping is not implemented yet; current roster publish advertises
    /// every local capability to every workspace.
    Add {
        /// Agent slug ([A-Za-z0-9._-]).
        slug: String,
        /// Intended workspace assignment (repeatable). Currently triggers a
        /// roster republish; per-workspace scoping is not implemented yet.
        #[arg(long = "workspace", value_name = "WORKSPACE")]
        workspaces: Vec<String>,
        /// Harness bundle key from harnesses.json.
        #[arg(long, value_name = "BUNDLE")]
        harness: String,
        /// Optional harness-native named profile.
        #[arg(long, value_name = "PROFILE")]
        profile: Option<String>,
    },
    /// Assign an existing local agent to one or more workspaces. Per-workspace
    /// roster scoping is not implemented yet; this republished roster still
    /// advertises every local capability to every workspace.
    Assign {
        /// Agent slug (must already exist in the local keystore).
        slug: String,
        /// Workspace to assign to (repeatable; at least one required).
        #[arg(long = "workspace", value_name = "WORKSPACE", required = true)]
        workspaces: Vec<String>,
    },
    /// Remove a local agent. Its key file is parked at `<slug>.json.removed`
    /// (not deleted) so a mistake is recoverable; the agent stops being spawnable
    /// and stops being auto-trusted on next read.
    Remove {
        /// Agent slug to remove.
        slug: String,
    },
}

/// `channel add` targets. Exactly one of two shapes: a human member by id
/// (two positionals `<id> <channel>`) or an existing session pulled in
/// (`--session <npub|hex|current-handle> <channel>`). Session mode takes ONE positional
/// (the channel); human mode takes TWO. `--admin` is human-only; `--message`
/// posts a chat mentioning the brought-online session and is valid only with
/// `--session`.
#[derive(Args)]
pub(in crate::cli) struct AddArgs {
    /// Human mode: the member id (hex pubkey, npub, or nip05). Session mode: the
    /// channel-relative channel to add into.
    #[arg(value_name = "ID_OR_CHANNEL")]
    pub(in crate::cli::admin) first: Option<String>,
    /// Human mode only: the channel-relative channel (second positional).
    #[arg(value_name = "CHANNEL")]
    pub(in crate::cli::admin) second: Option<String>,
    /// Pull an exact existing session by npub/hex or its current handle.
    #[arg(long, value_name = "HANDLE", conflicts_with = "admin")]
    pub(in crate::cli::admin) session: Option<String>,
    /// Grant admin rather than member. Human target only.
    #[arg(long)]
    pub(in crate::cli::admin) admin: bool,
    /// Also post a chat line into the channel mentioning the brought-online
    /// session. Valid only with `--session`.
    #[arg(long, value_name = "TEXT")]
    pub(in crate::cli::admin) message: Option<String>,
}

/// Subgroup task channels under a root (child channels).
#[derive(Subcommand)]
pub(in crate::cli) enum ChannelAction {
    /// Add a member to a channel: a human by id or an existing session
    /// (`--session <npub|hex|current-handle>`).
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
        /// Keep the channel reader open and print new messages as they arrive.
        #[arg(long)]
        live: bool,
        /// Channel-relative channel name/path/id to read. Required when this
        /// session is joined to more than one channel; inferred only when exactly
        /// one joined channel exists.
        #[arg(long)]
        channel: Option<String>,
        /// Public reader identity (npub, hex pubkey, or handle) instead of resolving from the current
        /// PTY/harness process or root scan.
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
        /// Upload a file to Blossom and replace every `[LABEL]` in the message
        /// with its public URL. Repeat for multiple attachments.
        #[arg(
            long = "attach",
            value_name = "LABEL=FILE",
            value_parser = crate::attachment::parse_spec
        )]
        attachments: Vec<crate::attachment::Attachment>,
        /// Agent to tag in the message. Repeat to tag multiple agents. The
        /// visible `nostr:npub...` address prefix is added automatically.
        #[arg(long = "tag", value_name = "AGENT")]
        tags: Vec<String>,
        /// Publish mention-like `@agent` text literally when no --tag is used.
        #[arg(long)]
        force: bool,
        /// Channel-relative channel name/path/id to write to. Required when
        /// this session is joined to more than one channel; inferred only when
        /// exactly one joined channel exists.
        #[arg(long)]
        channel: Option<String>,
        /// Public sender identity (npub, hex pubkey, or handle) instead of resolving from the current
        /// PTY/harness process or root scan.
        #[arg(long)]
        session: Option<String>,
        /// Allow publishing a message longer than the default 600-character cap.
        #[arg(long)]
        long_message: bool,
        /// Block for up to SECONDS until a correlated reply arrives.
        #[arg(long, value_name = "SECONDS", value_parser = crate::cli::messaging::parse_wait_seconds)]
        wait: Option<u64>,
    },
    /// Reply to a specific channel message by short id.
    Reply {
        /// Short or full message/event id from a mention envelope.
        id: String,
        /// Reply body. Positional, or via --message, or piped on stdin.
        #[arg(value_name = "MESSAGE")]
        message: Option<String>,
        #[arg(long = "message", value_name = "MESSAGE")]
        message_flag: Option<String>,
        /// Upload a file to Blossom and replace every `[LABEL]` in the message
        /// with its public URL. Repeat for multiple attachments.
        #[arg(
            long = "attach",
            value_name = "LABEL=FILE",
            value_parser = crate::attachment::parse_spec
        )]
        attachments: Vec<crate::attachment::Attachment>,
        /// Public sender identity (npub, hex pubkey, or handle) instead of resolving from the current
        /// PTY/harness process or root scan.
        #[arg(long)]
        session: Option<String>,
        /// Allow publishing a message longer than the default 600-character cap.
        #[arg(long)]
        long_message: bool,
    },
    /// React to a specific channel message with an emoji (a non-disruptive ACK).
    /// Unlike a chat reply, a reaction NEVER interrupts the target's turn — it
    /// surfaces as compact awareness at their next turn start. Use it for a bare
    /// acknowledgement ("got it", 👍, ✅) instead of sending a chat line.
    React {
        /// Short or full message/event id from a mention envelope.
        id: String,
        /// The reaction emoji (e.g. 👍 ✅ 👀 🎉) or `+`/`-`.
        #[arg(value_name = "EMOJI")]
        emoji: String,
        /// Public reactor identity (npub, hex pubkey, or handle) instead of resolving from the current
        /// PTY/harness process or root scan.
        #[arg(long)]
        session: Option<String>,
    },
    /// Create a subgroup task channel and focus it. When run as an agent
    /// the new channel nests under your CURRENT channel by default, and the
    /// running session auto-joins it. If `--agent slug@backend-label` targets
    /// are named, one kind:9 orchestration event asks those backends to add
    /// their agents.
    Create {
        /// Channel-relative path to create, e.g. "support" or "epic.planning".
        /// Parent segments address the parent channel; the final segment is the
        /// new channel's durable human name.
        #[arg(value_name = "PATH")]
        path: String,
        /// Short, stable channel description (max 80 chars), not status text.
        #[arg(long, value_parser = crate::channel_about::parse_channel_about)]
        about: String,
        /// Optional, repeatable `slug@backend-label`, where `backend-label` is
        /// the target backend's config.json `backendName`. Omit to create an
        /// empty channel.
        #[arg(long = "agent", value_name = "SLUG@BACKEND")]
        agents: Vec<String>,
        /// Public session identity (npub, hex pubkey, or handle) to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// Edit metadata on an existing subgroup task channel.
    Edit {
        /// Channel name, channel-relative path, or opaque channel `h` value.
        channel: String,
        /// New durable channel description.
        #[arg(long, value_parser = crate::channel_about::parse_channel_about)]
        about: String,
        /// Public session identity (npub, hex pubkey, or handle) used as the channel-relative resolution anchor.
        #[arg(long)]
        session: Option<String>,
    },
    /// List the subgroup task channels under a workspace, or every workspace
    /// on the relay with `--all-workspaces`.
    List {
        /// Workspace slug. Defaults to the workspace resolved from the current
        /// directory. Ignored with `--all-workspaces`.
        #[arg(long = "workspace", value_name = "WORKSPACE")]
        workspace: Option<String>,
        /// List every workspace on the relay instead of one workspace's subgroup tree.
        #[arg(long = "all-workspaces", conflicts_with = "workspace")]
        workspaces: bool,
    },
    /// Register the current directory as a mosaico workspace. Maps
    /// the directory's basename as a slug in `~/.mosaico/workspaces.json` so a
    /// non-git directory resolves to a workspace. Refuses if the slug is already
    /// mapped to a different path; pass `--force` to overwrite. No-op if the
    /// slug already maps to this exact path.
    Init {
        /// Overwrite an existing slug->path mapping that points elsewhere.
        #[arg(long)]
        force: bool,
    },
    /// Join a channel for passive context and direct-mention delivery.
    Join {
        /// Channel name, channel-relative path, or opaque channel `h` value.
        channel: String,
        /// Public session identity (npub, hex pubkey, or handle) to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// Stop listening to a passively joined channel.
    Leave {
        /// Channel name, channel-relative path, or opaque channel `h` value.
        channel: String,
        /// Public session identity (npub, hex pubkey, or handle) to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// Mark a channel archived and remove all non-admin members.
    Archive {
        /// Channel name, channel-relative path, or opaque channel `h` value.
        channel: String,
        /// Public session identity (npub, hex pubkey, or handle) used as the channel-relative resolution anchor.
        #[arg(long)]
        session: Option<String>,
    },
    /// Switch the active channel for the current session to a different subgroup.
    Switch {
        /// Channel name, channel-relative path, or opaque channel `h` value.
        channel: String,
        /// Public session identity (npub, hex pubkey, or handle) to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
}

#[cfg(test)]
mod tests;

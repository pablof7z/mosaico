use super::super::search::ChannelSearchArgs;
use clap::{Args, Subcommand};

/// Every channel-taking argument requires a full absolute path
/// (`/workspace/child`) — never a bare name or opaque channel id.
/// Fast, same-process rejection instead of a daemon round trip.
pub(in crate::cli) fn parse_channel_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("channel must not be empty".to_string());
    }
    if !trimmed.starts_with('/') {
        return Err(format!(
            "channel must be a full path starting with \"/\", e.g. /workspace/child (got {raw:?})"
        ));
    }
    Ok(trimmed.to_string())
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
    /// full channel path (e.g. /workspace/child) to add into.
    #[arg(value_name = "ID_OR_CHANNEL")]
    pub(in crate::cli::admin) first: Option<String>,
    /// Human mode only: the full channel path (second positional).
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
        /// Full channel path (e.g. /workspace/child).
        /// Required when this session is joined to more than one channel;
        /// inferred only when exactly one joined channel exists. No inference
        /// is possible when the joined set is empty. Must already be joined
        /// when given explicitly.
        #[arg(long, value_parser = parse_channel_path)]
        channel: Option<String>,
        /// Public reader identity (npub, hex pubkey, or handle) instead of resolving from the current
        /// PTY/harness process or root scan.
        #[arg(long)]
        session: Option<String>,
    },
    /// Search messages already present in the local database.
    Search(ChannelSearchArgs),
    /// Send a chat line to a joined channel. Reads body from arg, --message, or stdin.
    Send {
        /// Message body. Positional, or via --message, or piped on stdin.
        #[arg(value_name = "MESSAGE")]
        message: Option<String>,
        #[arg(long = "message", value_name = "MESSAGE")]
        message_flag: Option<String>,
        /// Upload FILE to Blossom. Its supplied relative path is the bracket
        /// label; absent labels are appended to the message. Repeat for files.
        #[arg(
            long = "attach",
            value_name = "FILE",
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
        /// Full channel path (e.g. /workspace/child).
        /// Required when this session is joined to more than one channel;
        /// inferred only when exactly one joined channel exists. No inference
        /// is possible when the joined set is empty. Must already be joined
        /// when given explicitly.
        #[arg(long, value_parser = parse_channel_path)]
        channel: Option<String>,
        /// Public sender identity (npub, hex pubkey, or handle) instead of resolving from the current
        /// PTY/harness process or root scan.
        #[arg(long)]
        session: Option<String>,
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
        /// Upload FILE to Blossom. Its supplied relative path is the bracket
        /// label; absent labels are appended to the message. Repeat for files.
        #[arg(
            long = "attach",
            value_name = "FILE",
            value_parser = crate::attachment::parse_spec
        )]
        attachments: Vec<crate::attachment::Attachment>,
        /// Public sender identity (npub, hex pubkey, or handle) instead of resolving from the current
        /// PTY/harness process or root scan.
        #[arg(long)]
        session: Option<String>,
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
    /// Create one channel at an explicit path. When run as an agent, creation
    /// additively joins the new channel without leaving any existing channels.
    /// If `--agent slug@backend-label` targets are named, one kind:9
    /// orchestration event asks those backends to add their agents.
    Create {
        /// Full absolute path of the channel to create, e.g.
        /// "/workspace/epic/planning". The parent chain (everything but the
        /// last segment) must already exist; only the final segment is
        /// minted.
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
        /// Full channel path (e.g. /workspace/child).
        #[arg(value_parser = parse_channel_path)]
        channel: String,
        /// New durable channel description.
        #[arg(long, value_parser = crate::channel_about::parse_channel_about)]
        about: String,
        /// Public session identity (npub, hex pubkey, or handle) to act as instead of resolving the caller from the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// List the channel forest. By default, your own and joined workspaces are
    /// expanded while other known workspaces stay compact.
    List {
        /// Expand only this workspace. Useful outside an agent session.
        #[arg(
            long,
            value_name = "WORKSPACE",
            conflicts_with_all = ["all", "recursive"]
        )]
        workspace: Option<String>,
        /// Show every workspace root as a compact inventory.
        #[arg(short = 'a', long, conflicts_with = "recursive")]
        all: bool,
        /// Expand every known workspace and channel, including unjoined ones.
        #[arg(short = 'r', long)]
        recursive: bool,
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
        /// Full channel path (e.g. /workspace/child).
        #[arg(value_parser = parse_channel_path)]
        channel: String,
        /// Public session identity (npub, hex pubkey, or handle) to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// Stop listening to a passively joined channel.
    Leave {
        /// Full channel path (e.g. /workspace/child).
        #[arg(value_parser = parse_channel_path)]
        channel: String,
        /// Public session identity (npub, hex pubkey, or handle) to mutate instead of resolving the caller from
        /// the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
    /// Mark a channel archived and remove all non-admin members.
    Archive {
        /// Full channel path (e.g. /workspace/child).
        #[arg(value_parser = parse_channel_path)]
        channel: String,
        /// Public session identity (npub, hex pubkey, or handle) to act as instead of resolving the caller from the current PTY/harness process.
        #[arg(long)]
        session: Option<String>,
    },
}

#[cfg(test)]
mod tests;

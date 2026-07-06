use anyhow::Result;
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

#[derive(Args)]
pub(in crate::cli) struct InviteArgs {
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
}

pub(in crate::cli) async fn invite(args: InviteArgs) -> Result<()> {
    super::project_channels::invite_target(args.channel, args.agent, args.session).await
}

#[derive(Subcommand)]
pub(in crate::cli) enum ProjectAction {
    /// List all NIP-29 project groups on the relay.
    List,
    /// Initialize the current directory as a tenex-edge project. Registers the
    /// directory's basename as a slug in `~/.tenex-edge/projects.json`. Refuses
    /// if the slug is already mapped to a different path; pass `--force` to
    /// overwrite. No-op if the slug is already mapped to this exact path.
    Init {
        /// Overwrite an existing slug->path mapping that points elsewhere.
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
pub(in crate::cli) enum ChannelsAction {
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
    },
    /// Edit metadata on an existing subgroup task channel.
    Edit {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
        /// New durable channel description.
        #[arg(long, value_parser = crate::channel_about::parse_channel_about)]
        about: String,
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
    /// Mark a channel archived and remove all non-admin members.
    Archive {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
    },
    /// Switch the active channel for the current tmux pane to a different NIP-29 subgroup.
    Switch {
        /// Channel name, project-relative path, or opaque NIP-29 `h` value.
        channel: String,
    },
}

#[cfg(test)]
mod tests;

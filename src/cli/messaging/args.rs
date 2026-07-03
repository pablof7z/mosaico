use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli) enum ChatAction {
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
        /// Allow publishing a message longer than the default 600-character cap.
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
        #[arg(long)]
        channel: Option<String>,
    },
}

pub(in crate::cli) async fn chat(action: ChatAction) -> Result<()> {
    match action {
        ChatAction::Write {
            message,
            message_flag,
            channel,
            long_message,
        } => {
            let message = super::resolve_send_message_body(message_flag.or(message))?;
            super::chat_write(message, channel, long_message).await
        }
        ChatAction::Read {
            id,
            since,
            limit,
            offset,
            tail,
            live,
            channel,
        } => super::chat_read(id, since, limit, offset, tail, live, channel).await,
    }
}

#[derive(Args)]
pub(in crate::cli) struct PublishArgs {
    /// Proposal title.
    #[arg(long)]
    title: String,
    /// Proposal body (Markdown). Use "-" or omit to read from stdin.
    #[arg(long = "message", value_name = "BODY")]
    message: Option<String>,
    /// Stable addressable identifier (the kind:30023 `d` tag). Reuse the same
    /// value to publish a REVISION that supersedes a prior proposal at the same
    /// address. Omit to mint a fresh id (a new proposal).
    #[arg(long = "d", value_name = "IDENTIFIER")]
    d: Option<String>,
    /// My session id; if omitted, resolved from the current directory.
    #[arg(long)]
    session: Option<String>,
}

pub(in crate::cli) async fn publish(args: PublishArgs) -> Result<()> {
    let body = super::resolve_send_message_body(args.message)?;
    super::publish_proposal(args.title, body, args.d, args.session).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{error::ErrorKind, Parser};

    fn parse_err(args: &[&str]) -> clap::Error {
        match crate::cli::args::Cli::try_parse_from(args) {
            Ok(_) => panic!("expected parse failure for {args:?}"),
            Err(err) => err,
        }
    }

    #[test]
    fn chat_read_help_lists_channel_not_project_alias() {
        let err = parse_err(&["tenex-edge", "chat", "read", "--help"]);
        let help = err.to_string();

        assert!(help.contains("--channel <CHANNEL>"));
        assert!(!help.contains("--project <PROJECT>"));
    }

    #[test]
    fn chat_read_rejects_removed_project_alias() {
        let err = parse_err(&["tenex-edge", "chat", "read", "--project", "tmp"]);

        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn chat_read_channel_still_parses() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "chat",
            "read",
            "--channel",
            "ops",
        ])
        .unwrap();

        match cli.cmd {
            crate::cli::args::Cmd::Chat {
                action: ChatAction::Read { channel, .. },
            } => assert_eq!(channel.as_deref(), Some("ops")),
            _ => panic!("expected chat read command"),
        }
    }

    #[test]
    fn publish_args_parse_with_owner_args() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "publish",
            "--title",
            "T",
            "--message",
            "body",
            "--d",
            "proposal-1",
            "--session",
            "session-1",
        ])
        .unwrap();

        match cli.cmd {
            crate::cli::args::Cmd::Publish(args) => {
                assert_eq!(args.title, "T");
                assert_eq!(args.message.as_deref(), Some("body"));
                assert_eq!(args.d.as_deref(), Some("proposal-1"));
                assert_eq!(args.session.as_deref(), Some("session-1"));
            }
            _ => panic!("expected publish command"),
        }
    }
}

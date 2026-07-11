use anyhow::{bail, Result};
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(super) enum SessionAction {
    /// End a local session record.
    End(SessionEndArgs),
}

#[derive(Args)]
pub(super) struct SessionEndArgs {
    /// End the session this command is running inside.
    #[arg(long = "self", conflicts_with = "session")]
    pub(super) self_session: bool,
    /// Session id or alias to end.
    pub(super) session: Option<String>,
}

pub(super) fn session(action: SessionAction) -> Result<()> {
    match action {
        SessionAction::End(args) => end(args),
    }
}

fn end(args: SessionEndArgs) -> Result<()> {
    let session = match (args.self_session, args.session) {
        (true, None) => self_session_anchor()?,
        (false, Some(session)) => session,
        (false, None) => bail!("provide a session id or use `--self`"),
        (true, Some(_)) => unreachable!("clap conflicts_with prevents this"),
    };
    super::session_end(session)
}

fn self_session_anchor() -> Result<String> {
    super::pty_session_env()
        .or_else(|| {
            std::env::var("TENEX_EDGE_SESSION")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "`tenex-edge my session end --self` must run inside a tenex-edge PTY session"
            )
        })
}

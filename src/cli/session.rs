use anyhow::{bail, Result};
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(super) enum SessionAction {
    /// End a local session record (metadata only; the hosted process keeps running).
    End(SessionEndArgs),
    /// Kill this session's hosted process and mark it offline.
    Kill(SessionKillArgs),
}

#[derive(Args)]
pub(super) struct SessionEndArgs {
    /// End the session this command is running inside.
    #[arg(long = "self", conflicts_with = "session")]
    pub(super) self_session: bool,
    /// Session id or alias to end.
    pub(super) session: Option<String>,
}

#[derive(Args)]
pub(super) struct SessionKillArgs {
    /// Kill the session this command is running inside. Required: `session
    /// kill` never accepts a target other than the caller's own session, so
    /// agents may only kill themselves.
    #[arg(long = "self", required = true)]
    pub(super) self_session: bool,
}

pub(super) fn session(action: SessionAction) -> Result<()> {
    match action {
        SessionAction::End(args) => end(args),
        SessionAction::Kill(args) => kill(args),
    }
}

fn end(args: SessionEndArgs) -> Result<()> {
    let session = match (args.self_session, args.session) {
        (true, None) => self_session_anchor("end")?,
        (false, Some(session)) => session,
        (false, None) => bail!("provide a session id or use `--self`"),
        (true, Some(_)) => unreachable!("clap conflicts_with prevents this"),
    };
    super::session_end(session)
}

fn kill(args: SessionKillArgs) -> Result<()> {
    // Clap's `required = true` on `--self` already enforces this, but the
    // check documents the invariant and stays correct if that constraint is
    // ever relaxed: `session kill` never accepts a target other than the
    // caller's own session.
    if !args.self_session {
        bail!("`tenex-edge my session kill` requires `--self`");
    }
    let session = self_session_anchor("kill")?;
    super::session_kill(session)
}

fn self_session_anchor(verb: &str) -> Result<String> {
    super::pty_session_env()
        .or_else(|| {
            std::env::var("TENEX_EDGE_SESSION")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "`tenex-edge my session {verb} --self` must run inside a tenex-edge PTY session"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;

    fn outside_any_session() -> EnvGuard {
        let mut g = EnvGuard::set("TENEX_EDGE_PTY_SESSION", "");
        g.set_var("TENEX_EDGE_SESSION", "");
        g
    }

    #[test]
    fn kill_requires_self_flag() {
        let err = kill(SessionKillArgs {
            self_session: false,
        })
        .unwrap_err();

        assert!(err.to_string().contains("requires `--self`"));
    }

    #[test]
    fn kill_outside_session_reports_clear_diagnostic() {
        let _env = outside_any_session();

        let err = kill(SessionKillArgs { self_session: true }).unwrap_err();

        assert_eq!(
            err.to_string(),
            "`tenex-edge my session kill --self` must run inside a tenex-edge PTY session"
        );
    }

    #[test]
    fn end_self_outside_session_reports_clear_diagnostic() {
        let _env = outside_any_session();

        let err = end(SessionEndArgs {
            self_session: true,
            session: None,
        })
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "`tenex-edge my session end --self` must run inside a tenex-edge PTY session"
        );
    }

    #[test]
    fn end_without_self_or_session_is_rejected() {
        let err = end(SessionEndArgs {
            self_session: false,
            session: None,
        })
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("provide a session id or use `--self`"));
    }
}

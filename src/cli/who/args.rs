use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub(in crate::cli) struct WhoArgs {
    /// Workspace slug; defaults to the workspace resolved from current directory.
    #[arg(long = "workspace", value_name = "WORKSPACE")]
    workspace: Option<String>,
    /// Show the human fabric view across all workspaces.
    #[arg(long = "all-workspaces")]
    all_workspaces: bool,
    /// Keep a full-screen live view open, refreshing automatically.
    #[arg(long)]
    live: bool,
    /// List this machine's expired (dead/old) sessions by public handle so you can
    /// resume one, instead of the live fabric snapshot.
    #[arg(long, conflicts_with = "live")]
    expired: bool,
}

pub(in crate::cli) fn who(args: WhoArgs) -> Result<()> {
    if args.expired {
        super::who_expired()
    } else if args.live {
        super::who_live(args.workspace, args.all_workspaces)
    } else {
        super::who_once(args.workspace, args.all_workspaces)
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn who_all_workspaces_live_parse_with_owner_args() {
        let cli =
            crate::cli::args::Cli::try_parse_from(["mosaico", "who", "--all-workspaces", "--live"])
                .expect("who parses");

        match cli.cmd.expect("expected who command") {
            crate::cli::args::Cmd::Who(args) => {
                assert!(args.all_workspaces);
                assert!(args.live);
                assert!(!args.expired);
                assert_eq!(args.workspace, None);
            }
            _ => panic!("expected who command"),
        }
    }

    #[test]
    fn removed_who_all_roots_alias_stays_unavailable() {
        let err = match crate::cli::args::Cli::try_parse_from(["mosaico", "who", "--all-roots"]) {
            Ok(_) => panic!("legacy who alias must stay removed"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn who_expired_parses() {
        let cli = crate::cli::args::Cli::try_parse_from(["mosaico", "who", "--expired"])
            .expect("who --expired parses");
        match cli.cmd.expect("expected who command") {
            crate::cli::args::Cmd::Who(args) => assert!(args.expired),
            _ => panic!("expected who command"),
        }
    }

    /// `--expired` and `--live` are mutually exclusive (a one-shot listing vs a
    /// live refresh loop).
    #[test]
    fn who_expired_conflicts_with_live() {
        let err = crate::cli::args::Cli::try_parse_from(["mosaico", "who", "--expired", "--live"]);
        assert!(err.is_err(), "expired + live must conflict");
    }
}

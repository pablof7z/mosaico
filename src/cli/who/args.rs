use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub(in crate::cli) struct WhoArgs {
    #[arg(long)]
    root: Option<String>,
    /// Show agents across all root channels (overrides --root / cwd resolution).
    #[arg(long)]
    all_roots: bool,
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
        super::who_live(args.root, args.all_roots)
    } else {
        super::who_once(args.root, args.all_roots)
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn who_all_roots_live_parse_with_owner_args() {
        let cli =
            crate::cli::args::Cli::try_parse_from(["tenex-edge", "who", "--all-roots", "--live"])
                .expect("who parses");

        match cli.cmd {
            crate::cli::args::Cmd::Who(args) => {
                assert!(args.all_roots);
                assert!(args.live);
                assert!(!args.expired);
                assert_eq!(args.root, None);
            }
            _ => panic!("expected who command"),
        }
    }

    #[test]
    fn who_expired_parses() {
        let cli = crate::cli::args::Cli::try_parse_from(["tenex-edge", "who", "--expired"])
            .expect("who --expired parses");
        match cli.cmd {
            crate::cli::args::Cmd::Who(args) => assert!(args.expired),
            _ => panic!("expected who command"),
        }
    }

    /// `--expired` and `--live` are mutually exclusive (a one-shot listing vs a
    /// live refresh loop).
    #[test]
    fn who_expired_conflicts_with_live() {
        let err =
            crate::cli::args::Cli::try_parse_from(["tenex-edge", "who", "--expired", "--live"]);
        assert!(err.is_err(), "expired + live must conflict");
    }
}

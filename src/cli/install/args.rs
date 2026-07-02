use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub(in crate::cli) struct InstallArgs {
    #[arg(long)]
    all: bool,
    #[arg(long, value_name = "HARNESSES")]
    harness: Option<String>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    status: bool,
    #[arg(long)]
    uninstall: bool,
}

pub(super) struct InstallOpts {
    pub all: bool,
    pub harness: Option<String>,
    pub dry_run: bool,
    pub status: bool,
    pub uninstall: bool,
}

impl InstallArgs {
    fn into_opts(self) -> InstallOpts {
        InstallOpts {
            all: self.all,
            harness: self.harness,
            dry_run: self.dry_run,
            status: self.status,
            uninstall: self.uninstall,
        }
    }
}

pub(in crate::cli) async fn install(args: InstallArgs) -> Result<()> {
    super::install_with_opts(args.into_opts()).await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn install_flags_parse_with_owner_args() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "install",
            "--harness",
            "codex,claude-code",
            "--dry-run",
            "--status",
        ])
        .expect("install flags parse");

        match cli.cmd {
            crate::cli::args::Cmd::Install(args) => {
                assert_eq!(args.harness.as_deref(), Some("codex,claude-code"));
                assert!(args.dry_run);
                assert!(args.status);
                assert!(!args.all);
                assert!(!args.uninstall);
            }
            _ => panic!("expected install command"),
        }
    }
}

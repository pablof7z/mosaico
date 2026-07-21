use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub(in crate::cli) struct SetupArgs {
    #[arg(long)]
    all: bool,
    #[arg(long, value_name = "HARNESSES")]
    harness: Option<String>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    status: bool,
    /// Use one or more remote relay URLs. Repeat for multiple relays.
    #[arg(long, value_name = "WS_URL", conflicts_with = "local_relay")]
    relay: Vec<String>,
    /// Configure and start the bundled relay on 127.0.0.1:9888.
    #[arg(long, conflicts_with = "relay")]
    local_relay: bool,
    /// Device label published for agents running on this host.
    #[arg(long, value_name = "LABEL")]
    host_label: Option<String>,
    /// Comma-separated operator public keys (hex or npub).
    #[arg(
        long,
        value_name = "HEX_OR_NPUB,...",
        conflicts_with = "clear_operators"
    )]
    operator_pubkeys: Option<String>,
    /// Remove every configured operator public key.
    #[arg(long)]
    clear_operators: bool,
    /// Read the optional human CLI signing key from a file.
    #[arg(long, value_name = "PATH", conflicts_with = "clear_operator_nsec")]
    operator_nsec_file: Option<PathBuf>,
    /// Remove the optional human CLI signing key while preserving backend identity.
    #[arg(long)]
    clear_operator_nsec: bool,
    /// Relay used for kind:0 profile lookup and publishing.
    #[arg(long, value_name = "WS_URL")]
    indexer_relay: Option<String>,
    /// Whether human-started sessions receive their own subgroup.
    #[arg(long, value_name = "BOOL")]
    per_session_rooms: Option<bool>,
    /// Configure a local relay without starting it after setup.
    #[arg(long, requires = "local_relay")]
    no_start_local_relay: bool,
}

#[derive(Default)]
pub(super) struct InstallOpts {
    pub all: bool,
    pub harness: Option<String>,
    pub dry_run: bool,
    pub status: bool,
    pub uninstall: bool,
    pub relay: Vec<String>,
    pub local_relay: bool,
    pub host_label: Option<String>,
    pub operator_pubkeys: Option<String>,
    pub clear_operators: bool,
    pub operator_nsec_file: Option<PathBuf>,
    pub clear_operator_nsec: bool,
    pub indexer_relay: Option<String>,
    pub per_session_rooms: Option<bool>,
    pub no_start_local_relay: bool,
}

impl SetupArgs {
    fn into_opts(self) -> InstallOpts {
        InstallOpts {
            all: self.all,
            harness: self.harness,
            dry_run: self.dry_run,
            status: self.status,
            uninstall: false,
            relay: self.relay,
            local_relay: self.local_relay,
            host_label: self.host_label,
            operator_pubkeys: self.operator_pubkeys,
            clear_operators: self.clear_operators,
            operator_nsec_file: self.operator_nsec_file,
            clear_operator_nsec: self.clear_operator_nsec,
            indexer_relay: self.indexer_relay,
            per_session_rooms: self.per_session_rooms,
            no_start_local_relay: self.no_start_local_relay,
        }
    }
}

impl InstallOpts {
    pub(super) fn uninstall(dry_run: bool) -> Self {
        Self {
            all: true,
            dry_run,
            uninstall: true,
            ..Self::default()
        }
    }
}

pub(in crate::cli) async fn setup(args: SetupArgs) -> Result<()> {
    super::install_with_opts(args.into_opts()).await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn setup_flags_parse_with_owner_args() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "mosaico",
            "setup",
            "--harness",
            "codex,claude-code",
            "--dry-run",
            "--status",
            "--relay",
            "wss://relay.example",
            "--host-label",
            "laptop",
            "--operator-pubkeys",
            "abc,def",
            "--operator-nsec-file",
            "/tmp/operator.nsec",
            "--indexer-relay",
            "wss://indexer.example",
            "--per-session-rooms",
            "true",
        ])
        .expect("install flags parse");

        match cli.cmd.expect("expected install command") {
            crate::cli::args::Cmd::Setup(args) => {
                assert_eq!(args.harness.as_deref(), Some("codex,claude-code"));
                assert!(args.dry_run);
                assert!(args.status);
                assert!(!args.all);
                assert_eq!(args.relay, ["wss://relay.example"]);
                assert_eq!(args.host_label.as_deref(), Some("laptop"));
                assert_eq!(args.operator_pubkeys.as_deref(), Some("abc,def"));
                assert_eq!(
                    args.operator_nsec_file.as_deref(),
                    Some(std::path::Path::new("/tmp/operator.nsec"))
                );
                assert_eq!(args.indexer_relay.as_deref(), Some("wss://indexer.example"));
                assert_eq!(args.per_session_rooms, Some(true));
            }
            _ => panic!("expected setup command"),
        }
    }

    #[test]
    fn local_and_remote_relay_modes_conflict() {
        let error = crate::cli::args::Cli::try_parse_from([
            "mosaico",
            "setup",
            "--local-relay",
            "--relay",
            "wss://relay.example",
        ])
        .err()
        .expect("local and remote relay modes should conflict");

        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

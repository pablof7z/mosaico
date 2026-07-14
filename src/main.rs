use clap::Parser;
use tenex_edge::cli::{self, Cli};
use tenex_edge::command_forensics::CommandCallLog;

fn main() {
    // rig-core's reqwest pulls a second rustls crypto provider (aws-lc-rs)
    // alongside nostr-sdk's (ring), so rustls 0.23 can't auto-pick one and panics
    // on the first TLS handshake. Install an explicit process-wide default.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let argv = std::env::args().collect::<Vec<_>>();
    let command_log = CommandCallLog::start(&argv);

    // `--help --all` (in any order) prints the full help with hidden subcommands
    // revealed. clap intercepts `--help` before our code runs, so detect it here.
    if argv.len() > 1
        && argv[1..].iter().any(|a| a == "--all")
        && argv[1..].iter().any(|a| a == "--help" || a == "-h")
    {
        cli::print_help_all();
        std::process::exit(0);
    }

    // Bare invocation and top-level `--help` / `-h` (without `--all`) print the
    // same context-sensitive help: operator commands (`who`, `mgmt`, `launch`)
    // are shown only outside an agent context. Internal/debug commands stay
    // hidden; use `--all` for those. Only intercept top-level help so subcommand
    // help (`tenex-edge who --help`, etc.) still goes through clap normally.
    if argv.len() == 1 || matches!(argv.get(1).map(String::as_str), Some("--help" | "-h")) {
        cli::print_help_contextual();
        std::process::exit(0);
    }

    let cli = match Cli::try_parse_from(argv.clone()) {
        Ok(cli) => cli,
        Err(err) => {
            command_log.finish_clap_error(&err);
            let code = err.exit_code();
            let _ = err.print();
            std::process::exit(code);
        }
    };
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            command_log.finish_runtime_error(&format!("failed to start runtime: {e}"));
            eprintln!("tenex-edge: failed to start runtime: {e}");
            std::process::exit(1);
        }
    };
    let result = rt.block_on(cli::run(cli));
    command_log.finish_result(&result);
    if let Err(e) = result {
        eprintln!("tenex-edge: {e:#}");
        std::process::exit(1);
    }
}

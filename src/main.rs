use clap::Parser;
use mosaico::cli::{self, Cli};
use mosaico::command_forensics::CommandCallLog;

fn main() {
    let mut argv = std::env::args().collect::<Vec<_>>();
    // Harness callbacks are latency-sensitive and explicitly fail open. When
    // no daemon socket exists, return before Clap, Tokio, TLS, hook forensics,
    // or process discovery page in the full application. Require the complete
    // hook shape so malformed/manual invocations still receive normal errors.
    if inactive_hook_fast_path(&argv) {
        return;
    }

    // Select the crypto provider before the first TLS handshake.
    let _ = rustls::crypto::ring::default_provider().install_default();

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

    // Explicit top-level help stays context-sensitive. Only intercept top-level
    // help so subcommand help (`mosaico who --help`, etc.) still goes through
    // clap normally.
    if matches!(argv.get(1).map(String::as_str), Some("--help" | "-h")) {
        cli::print_help_contextual();
        std::process::exit(0);
    }

    // Bare mosaico is the primary operator flow: route it exactly through the
    // canonical agents command when a harness integration exists. A binary with
    // no installed integration gives setup guidance without starting a daemon.
    if argv.len() == 1 {
        match cli::install::route_bare_invocation() {
            Ok(true) => argv.push("agents".to_string()),
            Ok(false) => return,
            Err(error) => {
                command_log.finish_result(&Err(anyhow::anyhow!(error.to_string())));
                eprintln!("mosaico: {error:#}");
                std::process::exit(1);
            }
        }
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
            eprintln!("mosaico: failed to start runtime: {e}");
            std::process::exit(1);
        }
    };
    let result = rt.block_on(cli::run(cli));
    command_log.finish_result(&result);
    if let Err(e) = result {
        eprintln!("mosaico: {e:#}");
        std::process::exit(1);
    }
}

fn inactive_hook_fast_path(argv: &[String]) -> bool {
    let is_hook = argv.get(1).map(String::as_str) == Some("harness")
        && argv.get(2).map(String::as_str) == Some("hook")
        && argv
            .get(3)
            .is_some_and(|host| !host.is_empty() && host != "help")
        && argv
            .windows(2)
            .any(|pair| pair[0] == "--type" && !pair[1].is_empty());
    is_hook && !mosaico::daemon::socket_path().exists()
}

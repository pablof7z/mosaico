use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand)]
pub(in crate::cli) enum HarnessAction {
    /// Handle a hook event from a supported agent harness.
    /// Reads hook JSON from stdin; emits context to inject into the model (if any).
    /// Usage: `mosaico harness hook <name> --type <hook-type>`
    /// Always exits 0 — a hook failure (daemon down, config missing, RPC
    /// timeout, …) is fabric plumbing, never something to surface to the
    /// harness or inject into the agent's context.
    Hook {
        /// Hook-capable harness name: claude-code, codex, opencode, grok, …
        /// Run with name "help" to list known harnesses.
        harness: String,
        /// Hook type the harness fires: session-start, user-prompt-submit,
        /// post-tool-use, stop, session-end.
        #[arg(long = "type")]
        hook_type: String,
    },
}

impl HarnessAction {
    pub(in crate::cli) fn is_hook(&self) -> bool {
        matches!(self, Self::Hook { .. })
    }
}

pub(in crate::cli) async fn harness(action: HarnessAction) -> Result<()> {
    match action {
        HarnessAction::Hook { harness, hook_type } => {
            // Hooks fire on every turn of an unrelated harness session. An error
            // here (daemon down, config missing, RPC failure, …) must NEVER
            // surface as a nonzero exit or an injected error blob — that would
            // pollute the agent's context with fabric plumbing it didn't ask
            // about. Log it for our own debugging and always exit clean.
            if let Err(e) = super::hooks::hook_run(harness, hook_type).await {
                eprintln!("[mosaico] hook error (ignored): {e:#}");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn harness_hook_args_parse() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "mosaico",
            "harness",
            "hook",
            "--type",
            "session-start",
            "claude-code",
        ])
        .expect("harness hook parses");
        match cli.cmd.expect("expected harness command") {
            crate::cli::args::Cmd::Harness {
                action: HarnessAction::Hook { hook_type, .. },
            } => assert_eq!(hook_type.as_str(), "session-start"),
            _ => panic!("expected harness hook command"),
        }
    }
}

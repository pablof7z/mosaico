use clap::{Args, Subcommand};

#[derive(Args)]
pub(in crate::cli) struct AgentsArgs {
    #[command(subcommand)]
    pub(super) action: Option<AgentAction>,
}

#[derive(Subcommand)]
pub(super) enum AgentAction {
    /// Print every configured agent and available native/default agent.
    List,
    /// Create or update a configured agent binding.
    Add {
        slug: String,
        #[arg(long, value_name = "HARNESS")]
        harness: String,
        #[arg(long, value_name = "PROFILE")]
        profile: Option<String>,
        #[arg(long, value_name = "PRESET")]
        preset: Option<String>,
    },
    /// Permanently delete a configured agent JSON file.
    Remove { slug: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn agents(args: &[&str]) -> AgentsArgs {
        let cli = crate::cli::args::Cli::try_parse_from(args).unwrap();
        match cli.cmd.expect("expected agents command") {
            crate::cli::args::Cmd::Agents(args) => args,
            _ => panic!("expected agents command"),
        }
    }

    #[test]
    fn management_subcommands_still_take_precedence() {
        let args = agents(&["mosaico", "agents", "list"]);
        assert!(matches!(args.action, Some(AgentAction::List)));
    }
}

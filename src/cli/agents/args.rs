use anyhow::{bail, Context as _, Result};
use clap::{Args, Subcommand};
use std::io::{self, Read as _};

#[derive(Args)]
#[command(
    args_conflicts_with_subcommands = true,
    subcommand_precedence_over_arg = true
)]
pub(in crate::cli) struct AgentsArgs {
    #[command(subcommand)]
    pub(super) action: Option<AgentAction>,
    /// Agent, native profile combination, or existing session. Omit to open
    /// the interactive agent picker.
    #[arg(index = 1)]
    pub(super) target: Option<String>,
    /// Opening user prompt for a fresh session. Use "-" to read from stdin.
    #[arg(index = 2, value_name = "PROMPT")]
    prompt: Option<String>,
    /// Workspace slug; defaults to the workspace resolved from the current directory.
    #[arg(long = "workspace", value_name = "WORKSPACE")]
    workspace: Option<String>,
    /// Channel name, or omit its value to open the channel picker.
    #[arg(long, num_args(0..=1), default_missing_value = "")]
    channel: Option<String>,
    /// Public name for this session.
    #[arg(long = "name", value_name = "NAME")]
    session_name: Option<String>,
}

impl AgentsArgs {
    pub(super) fn launch_request(self) -> Result<Option<crate::cli::launch_cli::LaunchRequest>> {
        let Some(agent) = self.target else {
            return Ok(None);
        };
        Ok(Some(crate::cli::launch_cli::LaunchRequest {
            agent,
            root: self.workspace,
            channel: self.channel,
            session_name: self.session_name,
            prompt: resolve_initial_prompt(self.prompt)?,
        }))
    }
}

fn resolve_initial_prompt(raw: Option<String>) -> Result<Option<String>> {
    match raw {
        Some(prompt) if prompt == "-" => read_stdin_prompt().map(Some),
        Some(prompt) if prompt.is_empty() => bail!("prompt must not be empty"),
        Some(prompt) => Ok(Some(prompt)),
        None => Ok(None),
    }
}

fn read_stdin_prompt() -> Result<String> {
    let mut prompt = String::new();
    io::stdin()
        .read_to_string(&mut prompt)
        .context("failed to read prompt from stdin")?;
    let prompt = strip_single_trailing_newline(prompt);
    if prompt.is_empty() {
        bail!("prompt from stdin was empty");
    }
    Ok(prompt)
}

fn strip_single_trailing_newline(mut value: String) -> String {
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    value
}

#[derive(Subcommand)]
pub(super) enum AgentAction {
    /// Print every configured agent and available native/default agent.
    List,
    /// Create or update a configured agent binding.
    Add {
        slug: String,
        #[arg(long, value_name = "BUNDLE")]
        harness: String,
        #[arg(long, value_name = "PROFILE")]
        profile: Option<String>,
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
        match cli.cmd {
            crate::cli::args::Cmd::Agents(args) => args,
            _ => panic!("expected agents command"),
        }
    }

    #[test]
    fn target_and_launch_options_parse_on_agents() {
        let args = agents(&[
            "mosaico",
            "agents",
            "codex",
            "hello",
            "--workspace",
            "mosaico",
            "--channel",
            "ops",
            "--name",
            "builder",
        ]);

        assert_eq!(args.target.as_deref(), Some("codex"));
        let request = args.launch_request().unwrap().unwrap();
        assert_eq!(request.prompt.as_deref(), Some("hello"));
        assert_eq!(request.root.as_deref(), Some("mosaico"));
        assert_eq!(request.channel.as_deref(), Some("ops"));
        assert_eq!(request.session_name.as_deref(), Some("builder"));
    }

    #[test]
    fn management_subcommands_still_take_precedence() {
        let args = agents(&["mosaico", "agents", "list"]);
        assert!(matches!(args.action, Some(AgentAction::List)));
        assert!(args.target.is_none());
    }

    #[test]
    fn removed_launch_command_is_rejected() {
        assert!(crate::cli::args::Cli::try_parse_from(["mosaico", "launch", "codex"]).is_err());
    }
}

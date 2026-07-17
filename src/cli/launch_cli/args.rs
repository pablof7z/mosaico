use anyhow::{bail, Context as _, Result};
use clap::Args;
use std::io::{self, Read as _};

#[derive(Args)]
pub(in crate::cli) struct LaunchArgs {
    /// Agent, native profile combination, or configured harness. Omit to choose
    /// from every available launch target.
    #[arg(index = 1)]
    slug: Option<String>,
    /// Opening user prompt to inject into the fresh session. Use "-" to read
    /// from stdin.
    #[arg(index = 2, value_name = "PROMPT")]
    prompt: Option<String>,
    /// Workspace slug; defaults to the workspace resolved from current directory.
    #[arg(long = "workspace", value_name = "WORKSPACE")]
    workspace: Option<String>,
    /// Channel name to scope this agent into; resolved to its opaque id and
    /// created if absent. Omit the value (`--channel` with no argument) to
    /// open an interactive fuzzy picker over all known rooms for the workspace.
    /// When per-session rooms are disabled (the default), omitting `--channel`
    /// entirely also opens the picker; with per-session rooms enabled, omitting
    /// it mints a fresh per-session room instead. The daemon's mosaicoPrivateKey
    /// adds the agent as a member; if the same derived pubkey is already in the
    /// group a fresh session produces a distinct key via a new anchor, acting
    /// as a second personality.
    #[arg(long, num_args(0..=1), default_missing_value = "")]
    channel: Option<String>,
    /// Public name for this session. With `codex`, `--name forensic-researcher`
    /// creates `forensic-researcher-codex`; existing names are rejected.
    #[arg(long = "name", value_name = "NAME")]
    session_name: Option<String>,
}

pub(super) struct LaunchRequest {
    pub(super) agent: String,
    pub(super) root: Option<String>,
    pub(super) channel: Option<String>,
    pub(super) session_name: Option<String>,
    pub(super) prompt: Option<String>,
}

pub(in crate::cli) async fn launch(args: LaunchArgs) -> Result<()> {
    let slug = match args.slug {
        Some(slug) => slug,
        None => match super::selection::select_available().await? {
            Some(slug) => slug,
            None => return Ok(()),
        },
    };
    let prompt = resolve_initial_prompt(args.prompt)?;
    super::verbs::launch(LaunchRequest {
        agent: slug,
        root: args.workspace,
        channel: args.channel,
        session_name: args.session_name,
        prompt,
    })
    .await
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

fn strip_single_trailing_newline(mut s: String) -> String {
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn launch_channel_tristate_is_explicit_contract() {
        let omitted = crate::cli::args::Cli::try_parse_from(["mosaico", "launch", "codex"])
            .expect("launch without channel parses");
        let picker =
            crate::cli::args::Cli::try_parse_from(["mosaico", "launch", "codex", "--channel"])
                .expect("launch with channel picker parses");
        let named = crate::cli::args::Cli::try_parse_from([
            "mosaico",
            "launch",
            "codex",
            "--channel",
            "ops",
        ])
        .expect("launch with named channel parses");

        let channel = |cli: crate::cli::args::Cli| match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => args.channel,
            _ => panic!("expected launch command"),
        };

        assert_eq!(channel(omitted), None);
        assert_eq!(channel(picker).as_deref(), Some(""));
        assert_eq!(channel(named).as_deref(), Some("ops"));
    }

    #[test]
    fn launch_without_agent_opens_available_target_selection() {
        let cli = crate::cli::args::Cli::try_parse_from(["mosaico", "launch"])
            .expect("launch without agent parses");
        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => assert!(args.slug.is_none()),
            _ => panic!("expected launch command"),
        }
    }

    #[test]
    fn launch_name_parses_as_a_public_session_name() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "mosaico",
            "launch",
            "codex",
            "--name",
            "forensic-researcher",
        ])
        .expect("launch with name parses");

        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => {
                assert_eq!(args.session_name.as_deref(), Some("forensic-researcher"));
            }
            _ => panic!("expected launch command"),
        }
    }

    #[test]
    fn launch_workspace_flag_parses() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "mosaico",
            "launch",
            "codex",
            "--workspace",
            "mosaico",
        ])
        .expect("launch --workspace parses");

        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => {
                assert_eq!(args.workspace.as_deref(), Some("mosaico"));
            }
            _ => panic!("expected launch command"),
        }
    }

    #[test]
    fn removed_launch_override_flags_are_rejected() {
        for flag in ["--harness", "--command", "--command-name", "--headless"] {
            assert!(crate::cli::args::Cli::try_parse_from([
                "mosaico", "launch", "claude", flag, "value"
            ])
            .is_err());
        }
    }
}

use anyhow::{bail, Context as _, Result};
use clap::Args;
use std::io::{self, Read as _};

#[derive(Args)]
pub(in crate::cli) struct LaunchArgs {
    /// Agent slug: "claude", "codex", "opencode", or a local custom agent.
    #[arg(index = 1)]
    slug: String,
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
    /// it mints a fresh per-session room instead. The daemon's tenexPrivateKey
    /// adds the agent as a member; if the same derived pubkey is already in the
    /// group a fresh session produces a distinct key via a new anchor, acting
    /// as a second personality.
    #[arg(long, num_args(0..=1), default_missing_value = "")]
    channel: Option<String>,
    /// Public name for this session. With `codex`, `--name forensic-researcher`
    /// creates `forensic-researcher-codex`; existing names are rejected.
    #[arg(long = "name", value_name = "NAME")]
    session_name: Option<String>,
    /// Override the entire launch command for this launch (shell-word split).
    /// Example: `-c 'ollama launch claude -- --dangerously-skip-permissions'`
    #[arg(short = 'c', long = "command", value_name = "COMMAND")]
    command_str: Option<String>,
    /// Select a named command from the agent file's `commands` list.
    #[arg(long = "command-name", value_name = "NAME", conflicts_with = "harness")]
    command_name: Option<String>,
    /// Launch through a named PTY bundle from harnesses.json (or a built-in
    /// PTY harness such as claude/codex/opencode/grok), bypassing the picker.
    #[arg(
        long = "harness",
        value_name = "BUNDLE",
        conflicts_with = "command_str"
    )]
    harness: Option<String>,
    /// Launch the agent's configured ACP/app-server bundle without attaching.
    #[arg(
        long = "headless",
        conflicts_with_all = ["harness", "command_str", "command_name"]
    )]
    headless: bool,
    /// Extra args passed after `--`; appended to the launch command.
    /// Example: `tenex-edge launch codex -- --yolo`
    #[arg(index = 3, last = true, value_name = "ARGS")]
    extra_args: Vec<String>,
}

pub(super) struct LaunchRequest {
    pub(super) agent: String,
    pub(super) root: Option<String>,
    pub(super) channel: Option<String>,
    pub(super) session_name: Option<String>,
    pub(super) command_name: Option<String>,
    pub(super) harness: Option<String>,
    pub(super) headless: bool,
    pub(super) override_command: Vec<String>,
    pub(super) extra_args: Vec<String>,
    pub(super) prompt: Option<String>,
}

pub(in crate::cli) async fn launch(args: LaunchArgs) -> Result<()> {
    let prompt = resolve_initial_prompt(args.prompt)?;
    let override_command = args
        .command_str
        .map(|s| shlex::split(&s).unwrap_or_else(|| vec![s]))
        .unwrap_or_default();
    super::verbs::launch(LaunchRequest {
        agent: args.slug,
        root: args.workspace,
        channel: args.channel,
        session_name: args.session_name,
        command_name: args.command_name,
        harness: args.harness,
        headless: args.headless,
        override_command,
        extra_args: args.extra_args,
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
        let omitted = crate::cli::args::Cli::try_parse_from(["tenex-edge", "launch", "codex"])
            .expect("launch without channel parses");
        let picker =
            crate::cli::args::Cli::try_parse_from(["tenex-edge", "launch", "codex", "--channel"])
                .expect("launch with channel picker parses");
        let named = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
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
    fn launch_command_name_parses_independently_from_override() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "launch",
            "codex",
            "--command-name",
            "safe",
        ])
        .expect("launch with command name parses");

        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => {
                assert_eq!(args.command_name.as_deref(), Some("safe"));
                assert!(args.command_str.is_none());
            }
            _ => panic!("expected launch command"),
        }
    }

    #[test]
    fn launch_name_parses_as_a_public_session_name() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
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
            "tenex-edge",
            "launch",
            "codex",
            "--workspace",
            "tenex-edge",
        ])
        .expect("launch --workspace parses");

        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => {
                assert_eq!(args.workspace.as_deref(), Some("tenex-edge"));
            }
            _ => panic!("expected launch command"),
        }
    }

    #[test]
    fn launch_harness_override_parses() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "launch",
            "claude",
            "--harness",
            "codex-yolo",
        ])
        .expect("launch --harness parses");

        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => {
                assert_eq!(args.harness.as_deref(), Some("codex-yolo"));
            }
            _ => panic!("expected launch command"),
        }
    }

    #[test]
    fn launch_harness_conflicts_with_command_overrides() {
        for args in [
            vec![
                "tenex-edge",
                "launch",
                "claude",
                "--harness",
                "codex",
                "--command",
                "claude",
            ],
            vec![
                "tenex-edge",
                "launch",
                "claude",
                "--harness",
                "codex",
                "--command-name",
                "safe",
            ],
        ] {
            assert!(crate::cli::args::Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn launch_headless_bypass_parses_and_conflicts_with_pty_overrides() {
        let cli =
            crate::cli::args::Cli::try_parse_from(["tenex-edge", "launch", "claude", "--headless"])
                .expect("launch --headless parses");
        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => assert!(args.headless),
            _ => panic!("expected launch command"),
        }

        assert!(crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "launch",
            "claude",
            "--headless",
            "--harness",
            "claude",
        ])
        .is_err());
    }

    #[test]
    fn launch_prompt_parses_before_forwarded_args() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "launch",
            "codex",
            "check on the deploy",
            "--",
            "--yolo",
        ])
        .expect("launch with prompt and forwarded args parses");

        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => {
                assert_eq!(args.prompt.as_deref(), Some("check on the deploy"));
                assert_eq!(args.extra_args, vec!["--yolo"]);
            }
            _ => panic!("expected launch command"),
        }
    }

    #[test]
    fn launch_forwarded_args_do_not_become_prompt() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "tenex-edge",
            "launch",
            "codex",
            "--",
            "--yolo",
        ])
        .expect("launch with only forwarded args parses");

        match cli.cmd {
            crate::cli::args::Cmd::Launch(args) => {
                assert_eq!(args.prompt.as_deref(), None);
                assert_eq!(args.extra_args, vec!["--yolo"]);
            }
            _ => panic!("expected launch command"),
        }
    }
}

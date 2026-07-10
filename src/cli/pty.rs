use anyhow::{Context, Result};
use clap::{Args, Subcommand};

mod list;

#[derive(Subcommand)]
pub(in crate::cli) enum PtyAction {
    /// List experimental portable-pty sessions.
    List,
    /// Attach to a session id or socket path.
    Attach { id: String },
    /// Inject text into a session id or socket path.
    Inject(InjectArgs),
    /// Resize a session PTY.
    Resize(ResizeArgs),
    /// Kill a session's child process.
    Kill { id: String },
}

#[derive(Args)]
pub(in crate::cli) struct InjectArgs {
    id: String,
    #[arg(long)]
    bracketed: bool,
    #[arg(long)]
    no_submit: bool,
    text: Option<String>,
}

#[derive(Args)]
pub(in crate::cli) struct ResizeArgs {
    id: String,
    #[arg(long)]
    rows: u16,
    #[arg(long)]
    cols: u16,
}

#[derive(Args)]
pub(in crate::cli) struct PtySupervisorArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    socket: std::path::PathBuf,
    #[arg(long)]
    cwd: std::path::PathBuf,
    #[arg(long)]
    agent: String,
    #[arg(long)]
    channel: Option<String>,
    #[arg(long)]
    ephemeral: bool,
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

pub(in crate::cli) fn pty(action: PtyAction) -> Result<()> {
    match action {
        PtyAction::List => list::list(),
        PtyAction::Attach { id } => crate::pty::attach(&resolve_endpoint_id(&id)),
        PtyAction::Inject(args) => {
            let text = match args.text {
                Some(text) => text,
                None => {
                    let mut text = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                        .context("reading inject text from stdin")?;
                    text
                }
            };
            crate::pty::inject(
                &resolve_endpoint_id(&args.id),
                &text,
                args.bracketed,
                !args.no_submit,
            )
        }
        PtyAction::Resize(args) => {
            crate::pty::resize(&resolve_endpoint_id(&args.id), args.rows, args.cols)
        }
        PtyAction::Kill { id } => crate::pty::kill(&resolve_endpoint_id(&id)),
    }
}

fn resolve_endpoint_id(id: &str) -> String {
    crate::daemon::blocking::call_no_spawn("pty_attach", serde_json::json!({ "session": id }))
        .ok()
        .and_then(|v| v["pty_id"].as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| id.to_string())
}

pub(in crate::cli) fn pty_supervisor(args: PtySupervisorArgs) -> Result<()> {
    crate::pty::run_supervisor(crate::pty::SupervisorArgs {
        id: args.id,
        socket: args.socket,
        cwd: args.cwd,
        agent: args.agent,
        channel: args.channel,
        ephemeral: args.ephemeral,
        command: args.command,
    })
}

use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub(in crate::cli) struct DispatchArgs {
    /// Agent/backend target as agent[@backend-label].
    #[arg(index = 1)]
    target: String,
    /// Target workspace/root channel where the new session runs.
    #[arg(long)]
    workspace: String,
    /// Fully-qualified channel to join. Repeat to join several channels.
    #[arg(long = "channel")]
    channels: Vec<String>,
    /// Message to send after the new session ACKs. Use "-" to read stdin.
    #[arg(long)]
    message: Option<String>,
}

pub(in crate::cli) async fn dispatch(args: DispatchArgs) -> Result<()> {
    let message = super::messaging::resolve_send_message_body(args.message)?;
    let v = super::daemon_call_async(
        "dispatch",
        crate::cli::rpc_params(serde_json::json!({
            "target": args.target,
            "workspace": args.workspace,
            "channels": args.channels,
            "message": message,
        })),
    )
    .await?;
    let dispatch_id = v["dispatch_event_id"].as_str().unwrap_or("?");
    let message_id = v["message_event_id"].as_str().unwrap_or("?");
    let route = v["route_channel"].as_str().unwrap_or("?");
    println!(
        "dispatched {} in {} via {route}; message {}",
        crate::idref::event_short_id(dispatch_id),
        v["workspace"].as_str().unwrap_or("?"),
        crate::idref::event_short_id(message_id)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn dispatch_parses_repeated_channels() {
        let cli = crate::cli::args::Cli::try_parse_from([
            "mosaico",
            "dispatch",
            "codex@backend2",
            "--workspace",
            "project2",
            "--channel",
            "project2.qa",
            "--channel",
            "project1.bug-123",
            "--message",
            "investigate",
        ])
        .unwrap();

        match cli.cmd.expect("expected dispatch command") {
            crate::cli::args::Cmd::Dispatch(args) => {
                assert_eq!(args.target, "codex@backend2");
                assert_eq!(args.workspace, "project2");
                assert_eq!(args.channels, vec!["project2.qa", "project1.bug-123"]);
                assert_eq!(args.message.as_deref(), Some("investigate"));
            }
            _ => panic!("expected dispatch command"),
        }
    }
}

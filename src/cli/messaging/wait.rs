use super::*;
use anyhow::Context;
use clap::Args;

#[derive(Args)]
pub(in crate::cli) struct WaitArgs {
    /// Maximum number of seconds to wait.
    #[arg(value_name = "SECONDS", value_parser = parse_wait_seconds)]
    pub(in crate::cli) timeout_secs: u64,
    /// Narrow the wait to a joined channel. Repeat for several channels.
    #[arg(long = "channel", value_name = "CHANNEL")]
    pub(in crate::cli) channels: Vec<String>,
    /// Only return chat authored by this human or agent.
    #[arg(long = "from", value_name = "MEMBER")]
    pub(in crate::cli) from: Option<String>,
}

pub(in crate::cli) fn parse_wait_seconds(raw: &str) -> std::result::Result<u64, String> {
    let seconds = raw
        .parse::<u64>()
        .map_err(|_| format!("invalid wait duration {raw:?}: expected whole seconds"))?;
    if seconds == 0 {
        return Err("wait duration must be at least 1 second".to_string());
    }
    Ok(seconds)
}

pub(in crate::cli) async fn wait(args: WaitArgs) -> Result<()> {
    let params = crate::cli::rpc_params(serde_json::json!({
        "timeout_secs": args.timeout_secs,
        "channels": args.channels,
        "from": args.from,
    }));
    let result = daemon_call_async("channel_wait", params).await?;
    print_result(&result)
}

pub(super) async fn wait_for_reply(
    send_result: &serde_json::Value,
    timeout_secs: u64,
    session: Option<String>,
) -> Result<()> {
    let event_id = send_result["id"]
        .as_str()
        .context("channel send returned no id")?;
    let params = crate::cli::rpc_params(serde_json::json!({
        "timeout_secs": timeout_secs,
        "reply_to": event_id,
        "session": session,
    }));
    let result = daemon_call_async("channel_wait", params).await?;
    print_result(&result)
}

fn print_result(result: &serde_json::Value) -> Result<()> {
    println!(
        "{}",
        crate::channel_render::format_wait_result(result, crate::util::now_secs())?
    );
    Ok(())
}

#[cfg(test)]
#[path = "wait/tests.rs"]
mod tests;

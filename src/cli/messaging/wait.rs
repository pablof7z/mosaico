use super::*;
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
    let event_id = send_result["event_id"]
        .as_str()
        .context("channel send returned no event id")?;
    let params = crate::cli::rpc_params(serde_json::json!({
        "timeout_secs": timeout_secs,
        "reply_to": event_id,
        "from_pubkeys": send_result["mentioned_pubkeys"],
        "from_labels": send_result["mentioned_labels"],
        "session": session,
    }));
    let result = daemon_call_async("channel_wait", params).await?;
    print_result(&result)
}

fn print_result(result: &serde_json::Value) -> Result<()> {
    match result["outcome"].as_str() {
        Some("message") => {
            let message = &result["message"];
            println!("{}", render_wait_message(message, crate::util::now_secs()));
            Ok(())
        }
        Some("timeout") => {
            let seconds = result["timeout_secs"].as_u64().unwrap_or_default();
            let channels = result["channels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>();
            println!(
                "{}",
                crate::injection::render_agent_wait_timeout(seconds, &channels)
            );
            Ok(())
        }
        other => bail!("daemon returned invalid channel wait outcome {other:?}"),
    }
}

fn render_wait_message(message: &serde_json::Value, now: u64) -> String {
    let channel = message["channel"].as_str().unwrap_or_default();
    let from = message["from_ref"]
        .as_str()
        .or_else(|| message["from_slug"].as_str())
        .unwrap_or_default();
    let recipients = message["recipient_refs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|recipient| recipient.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let mut out = String::from("<mosaico>");
    let _ = write!(
        out,
        "\n  <channel ref=\"{}\">",
        crate::agent_xml::attr(channel)
    );
    crate::agent_xml::write_message(
        &mut out,
        4,
        &crate::agent_xml::MessageElement {
            event_id: message["event_id"].as_str().unwrap_or_default(),
            from,
            recipients: &recipients,
            attachment_dir: message["attachment_dir"].as_str().unwrap_or_default(),
            body: message["body"].as_str().unwrap_or_default(),
            created_at: message["created_at"].as_u64().unwrap_or_default(),
            now,
        },
    );
    out.push_str("\n  </channel>\n</mosaico>");
    out
}

#[cfg(test)]
#[path = "wait/tests.rs"]
mod tests;

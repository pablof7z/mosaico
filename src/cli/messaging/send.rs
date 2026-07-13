use super::*;

pub(in crate::cli) async fn channel_send(
    message: String,
    tags: Vec<String>,
    force: bool,
    channel: Option<String>,
    session: Option<String>,
    long_message: bool,
    wait: Option<u64>,
) -> Result<()> {
    let params = crate::cli::rpc_params(serde_json::json!({
        "message": message,
        "tags": tags,
        "force": force,
        "long_message": long_message,
        "session": session.clone(),
        // Explicit `--channel` is destination targeting only. Caller identity
        // still comes from the session anchors added by `rpc_params`.
        "channel": channel,
    }));
    let v = daemon_call_async("channel_send", params).await?;
    if let Some(timeout_secs) = wait {
        return super::wait::wait_for_reply(&v, timeout_secs, session).await;
    }
    let event_id = v["event_id"].as_str().unwrap_or("?");
    let labels = v["mentioned_labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|label| label.as_str())
        .collect::<Vec<_>>();
    if labels.is_empty() {
        println!("sent chat {}", event_short_id(event_id));
    } else {
        let labels = labels
            .iter()
            .map(|label| format!("@{label}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("sent chat {} tagging {labels}", event_short_id(event_id));
    }
    Ok(())
}

use super::*;

pub(in crate::cli) async fn channel_reply(
    id: String,
    message: String,
    attachments: Vec<crate::attachment::Attachment>,
    session: Option<String>,
) -> Result<()> {
    let params = crate::cli::rpc_params(serde_json::json!({
        "id": id,
        "message": message,
        "attachments": attachments,
        "session": session,
    }));
    let v = daemon_call_async("channel_reply", params).await?;
    let event_id = v["event_id"].as_str().unwrap_or("?");
    let reply_to = v["reply_to"].as_str().unwrap_or("?");
    println!(
        "sent reply {} to {}",
        crate::util::short_id(event_id),
        crate::util::short_id(reply_to)
    );
    super::notices::print_recipient_reminders(&v)?;
    Ok(())
}

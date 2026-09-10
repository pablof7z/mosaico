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
    let id = v["id"].as_str().unwrap_or("?");
    println!("sent reply {id}");
    super::notices::print_send_coaching(&v)?;
    Ok(())
}

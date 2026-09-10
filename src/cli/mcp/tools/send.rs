use super::*;

pub(super) async fn channel_send(
    args: &Value,
    identity: &Value,
    allow_local_attachments: bool,
) -> Result<String> {
    validate_channel_send_args(args, allow_local_attachments)?;
    let wait_seconds = wait::send_timeout(args)?;
    if let Some(reply_to) = opt_string(args, "reply_to") {
        if wait_seconds.is_some() {
            anyhow::bail!("seconds is only valid when sending a new message");
        }
        let params = json!({
            "id": reply_to,
            "message": required_string(args, "message")?,
            "attachments": attachment_specs(args, allow_local_attachments)?,
        });
        let v = daemon_identity("channel_reply", params, identity).await?;
        let id = v["id"].as_str().unwrap_or("?");
        let notices =
            crate::channel_render::send_notice_lines(&v, crate::channel_render::NoticeHint::Tool)?;
        let mut lines = vec![format!("sent reply {id}")];
        lines.extend(notices);
        return Ok(lines.join("\n"));
    }
    let send = daemon_identity(
        "channel_send",
        channel_send_params(args, allow_local_attachments)?,
        identity,
    )
    .await?;
    let id = send["id"].as_str().unwrap_or("?");
    let notices =
        crate::channel_render::send_notice_lines(&send, crate::channel_render::NoticeHint::Tool)?;
    let Some(timeout_seconds) = wait_seconds else {
        let mut lines = vec![format!("sent chat {id}")];
        lines.extend(notices);
        return Ok(lines.join("\n"));
    };
    let outcome = wait::for_reply(&send, timeout_seconds, identity).await?;
    let wait_text = crate::channel_render::format_wait_result(&outcome, crate::util::now_secs())?;
    let mut lines = vec![format!("sent chat {id}")];
    lines.extend(notices);
    lines.push(wait_text);
    Ok(lines.join("\n"))
}

pub(super) fn channel_send_params(args: &Value, allow_local_attachments: bool) -> Result<Value> {
    validate_channel_send_args(args, allow_local_attachments)?;
    Ok(json!({
        "message": required_string(args, "message")?,
        "attachments": attachment_specs(args, allow_local_attachments)?,
        "tags": args.get("tags").and_then(Value::as_array).cloned().unwrap_or_default(),
        "force": args.get("force").and_then(Value::as_bool).unwrap_or(false),
        "channel": opt_string(args, "channel"),
        "wait_intent": args.get("seconds").is_some(),
    }))
}

fn validate_channel_send_args(args: &Value, allow_local_attachments: bool) -> Result<()> {
    const ALLOWED: &[&str] = &["message", "tags", "force", "channel", "seconds", "reply_to"];
    let object = args
        .as_object()
        .context("mosaico.channel_send arguments must be an object")?;
    if let Some(unknown) = object.keys().find(|key| {
        !ALLOWED.contains(&key.as_str())
            && !(allow_local_attachments && key.as_str() == "attachments")
    }) {
        anyhow::bail!("unsupported mosaico.channel_send argument {unknown:?}");
    }
    Ok(())
}

pub(super) fn attachment_specs(
    args: &Value,
    allow_local_attachments: bool,
) -> Result<Vec<crate::attachment::Attachment>> {
    let Some(values) = args.get("attachments") else {
        return Ok(Vec::new());
    };
    if !allow_local_attachments {
        anyhow::bail!("attachments are available only to local native harness integrations");
    }
    let parsed = values
        .as_array()
        .context("attachments must be an array of FILE or LABEL=FILE strings")?
        .iter()
        .map(|value| {
            let raw = value
                .as_str()
                .context("attachments entries must be FILE or LABEL=FILE strings")?;
            crate::attachment::parse_spec(raw).map_err(anyhow::Error::msg)
        })
        .collect::<Result<Vec<_>>>()?;
    crate::attachment::canonicalize(parsed)
}

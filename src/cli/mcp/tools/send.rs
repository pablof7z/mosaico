use super::*;

pub(super) async fn channel_send(
    args: &Value,
    identity: &Value,
    allow_local_attachments: bool,
) -> Result<Value> {
    validate_channel_send_args(args, allow_local_attachments)?;
    let wait_seconds = wait::send_timeout(args)?;
    if let Some(reply_to) = opt_string(args, "reply_to") {
        if wait_seconds.is_some() {
            anyhow::bail!("wait_seconds is only valid when sending a new message");
        }
        let params = with_session(
            json!({
                "id": reply_to,
                "message": required_string(args, "message")?,
                "attachments": attachment_specs(args, allow_local_attachments)?,
            }),
            args,
        );
        return daemon_identity("channel_reply", params, identity).await;
    }
    let send = daemon_identity(
        "channel_send",
        channel_send_params(args, allow_local_attachments)?,
        identity,
    )
    .await?;
    let Some(timeout_seconds) = wait_seconds else {
        return Ok(send);
    };
    let outcome = wait::for_reply(&send, timeout_seconds, args, identity).await?;
    Ok(json!({ "send": send, "wait": outcome }))
}

pub(super) fn channel_send_params(args: &Value, allow_local_attachments: bool) -> Result<Value> {
    validate_channel_send_args(args, allow_local_attachments)?;
    Ok(with_session(
        json!({
            "message": required_string(args, "message")?,
            "attachments": attachment_specs(args, allow_local_attachments)?,
            "tags": args.get("tags").and_then(Value::as_array).cloned().unwrap_or_default(),
            "force": args.get("force").and_then(Value::as_bool).unwrap_or(false),
            "channel": opt_string(args, "channel"),
            "wait_intent": args.get("wait_seconds").is_some(),
        }),
        args,
    ))
}

fn validate_channel_send_args(args: &Value, allow_local_attachments: bool) -> Result<()> {
    const ALLOWED: &[&str] = &[
        "message",
        "tags",
        "force",
        "channel",
        "session",
        "wait_seconds",
        "reply_to",
    ];
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

use super::*;

pub(super) async fn ambient(args: &Value, identity: &Value) -> Result<Value> {
    daemon_identity("channel_wait", ambient_params(args)?, identity).await
}

pub(super) fn send_timeout(args: &Value) -> Result<Option<u64>> {
    args.get("wait_seconds")
        .map(|_| required_timeout(args, "wait_seconds"))
        .transpose()
}

pub(super) async fn for_reply(
    send: &Value,
    timeout_seconds: u64,
    args: &Value,
    identity: &Value,
) -> Result<Value> {
    let event_id = send["event_id"]
        .as_str()
        .context("channel send returned no event id")?;
    daemon_identity(
        "channel_wait",
        reply_params(send, event_id, timeout_seconds, args),
        identity,
    )
    .await
}

fn ambient_params(args: &Value) -> Result<Value> {
    let timeout_seconds = required_timeout(args, "timeout_seconds")?;
    Ok(with_session(
        json!({
            "timeout_secs": timeout_seconds,
            "channels": string_array(args, "channels"),
            "from": opt_string(args, "from"),
        }),
        args,
    ))
}

fn reply_params(send: &Value, event_id: &str, timeout_seconds: u64, args: &Value) -> Value {
    with_session(
        json!({
            "timeout_secs": timeout_seconds,
            "reply_to": event_id,
            "from_pubkeys": string_array(send, "mentioned_pubkeys"),
            "from_labels": string_array(send, "mentioned_labels"),
        }),
        args,
    )
}

fn required_timeout(args: &Value, key: &str) -> Result<u64> {
    let seconds = args
        .get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("{key} must be a positive integer"))?;
    if seconds == 0 {
        anyhow::bail!("{key} must be at least 1 second");
    }
    Ok(seconds)
}

fn string_array(args: &Value, key: &str) -> Vec<Value> {
    args.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "wait/tests.rs"]
mod tests;

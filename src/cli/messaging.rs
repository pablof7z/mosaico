use super::*;
mod notices;
mod reply;
mod send;
pub(super) use reply::channel_reply;
pub(super) use send::{channel_send, ChannelSendRequest};
mod wait;
pub(super) use wait::{parse_wait_seconds, wait, WaitArgs};
#[cfg(test)]
mod envelope;
#[cfg(test)]
pub(super) use envelope::{format_envelope, EnvelopeView};

pub(super) async fn channel_react(id: String, emoji: String) -> Result<()> {
    let params = crate::cli::rpc_params(serde_json::json!({
        "id": id,
        "emoji": emoji.clone(),
    }));
    let v = daemon_call_async("channel_react", params).await?;
    let id = v["id"].as_str().unwrap_or("?");
    let target = v["target"].as_str().unwrap_or("?");
    let shown_emoji = v["emoji"].as_str().unwrap_or(&emoji);
    println!("reacted {} {} to {}", shown_emoji, id, target);
    Ok(())
}

pub(super) struct ChannelReadRequest {
    pub id: Option<String>,
    pub since: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub tail: bool,
    pub live: bool,
    pub channel: Option<String>,
}

pub(super) async fn channel_read(req: ChannelReadRequest) -> Result<()> {
    use std::io::IsTerminal as _;

    let since_ts = req.since.as_deref().map(super::admin::parse_since);
    let effective_tail = req.tail || req.since.is_none();
    let effective_limit = req.limit.or_else(|| {
        if req.since.is_none() || req.tail {
            Some(10)
        } else {
            None
        }
    });
    let use_color = std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal();

    let params = crate::cli::rpc_params(serde_json::json!({
        "id": req.id,
        "channel": req.channel,
        "since": since_ts,
        "limit": effective_limit,
        "offset": req.offset.unwrap_or(0),
        "tail": effective_tail,
        "live": req.live,
    }));
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client
        .stream("channel_read", params, move |item| {
            println!(
                "{}",
                crate::channel_render::render_channel_read_row(&item, use_color)
            );
        })
        .await
}

pub(super) fn resolve_send_message_body(raw: Option<String>) -> Result<String> {
    match raw {
        Some(message) if message == "-" => read_stdin_message(),
        Some(message) if message.is_empty() => bail!("message must not be empty"),
        Some(message) => Ok(expand_message_argument_newlines(message)),
        None => {
            if io::stdin().is_terminal() {
                bail!(
                    "missing message; pass it positionally, via --message, \
                     or pipe/heredoc it on stdin"
                );
            }
            read_stdin_message()
        }
    }
}

pub(super) fn expand_message_argument_newlines(message: String) -> String {
    message.replace("\\n", "\n")
}

fn read_stdin_message() -> Result<String> {
    let mut message = String::new();
    io::stdin()
        .read_to_string(&mut message)
        .context("failed to read message from stdin")?;
    let message = strip_single_trailing_newline(message);
    if message.is_empty() {
        bail!("message from stdin was empty");
    }
    Ok(message)
}

fn strip_single_trailing_newline(mut s: String) -> String {
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    s
}

#[cfg(test)]
mod tests;

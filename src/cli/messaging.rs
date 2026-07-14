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

pub(super) async fn channel_react(
    id: String,
    emoji: String,
    session: Option<String>,
) -> Result<()> {
    let params = crate::cli::rpc_params(serde_json::json!({
        "id": id,
        "emoji": emoji.clone(),
        "session": session,
    }));
    let v = daemon_call_async("channel_react", params).await?;
    let event_id = v["event_id"].as_str().unwrap_or("?");
    let target = v["target"].as_str().unwrap_or("?");
    let shown_emoji = v["emoji"].as_str().unwrap_or(&emoji);
    println!(
        "reacted {} {} to {}",
        shown_emoji,
        crate::util::short_id(event_id),
        crate::util::short_id(target)
    );
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
    pub session: Option<String>,
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
        "session": req.session,
        "since": since_ts,
        "limit": effective_limit,
        "offset": req.offset.unwrap_or(0),
        "tail": effective_tail,
        "live": req.live,
    }));
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client
        .stream("channel_read", params, move |item| {
            println!("{}", render_channel_read_row(&item, use_color));
        })
        .await
}

fn render_channel_read_row(item: &serde_json::Value, use_color: bool) -> String {
    let pubkey = item["from_pubkey"].as_str().unwrap_or_default();
    let slug = item["from_slug"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| pubkey_short(pubkey));
    let host = item["host"].as_str().filter(|s| !s.is_empty());
    let sender = match host {
        Some(host) => format!("<{slug}@{host}>"),
        // No host (e.g. a whitelisted human operator, who runs no session):
        // render bare rather than a misleading `<name@?>`.
        None => format!("<@{slug}>"),
    };
    let sender = color_by_pubkey(&sender, pubkey, use_color);
    let body = item["body"].as_str().unwrap_or_default().trim_end();
    let ts = item["created_at"].as_u64().unwrap_or(0);
    let mut text = format!("{sender} {body}");
    if item["truncated"].as_bool().unwrap_or(false) {
        if let Some(id) = item["event_id"].as_str().filter(|s| !s.is_empty()) {
            text.push_str(&format!(
                "\n[message truncated; run `tenex-edge channel read --id {id}`]"
            ));
        }
    }
    format!("{text} [{}]", format_local_datetime(ts))
}

fn color_by_pubkey(text: &str, pubkey: &str, use_color: bool) -> String {
    if !use_color || pubkey.is_empty() {
        return text.to_string();
    }
    let hash = pubkey.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| {
        acc.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(b)
    });
    match hash % 6 {
        0 => text.cyan().to_string(),
        1 => text.green().to_string(),
        2 => text.yellow().to_string(),
        3 => text.magenta().to_string(),
        4 => text.blue().to_string(),
        _ => text.red().to_string(),
    }
}

pub(super) fn resolve_send_message_body(raw: Option<String>) -> Result<String> {
    match raw {
        Some(message) if message == "-" => read_stdin_message(),
        Some(message) if message.is_empty() => bail!("message must not be empty"),
        Some(message) => Ok(message),
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

use super::*;
mod args;
pub(super) use args::{publish, PublishArgs};
mod send;
pub(super) use send::channel_send;
mod wait;
pub(super) use wait::{parse_wait_seconds, wait, WaitArgs};
pub(super) async fn channel_reply(
    id: String,
    message: String,
    session: Option<String>,
    long_message: bool,
) -> Result<()> {
    let params = crate::cli::rpc_params(serde_json::json!({
        "id": id,
        "message": message,
        "long_message": long_message,
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
    Ok(())
}

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

// ── publish ───────────────────────────────────────────────────────────────────

pub(super) async fn publish_proposal(
    title: String,
    body: String,
    d: Option<String>,
    session: Option<String>,
) -> Result<()> {
    let params = crate::cli::rpc_params(serde_json::json!({
        "title": title,
        "body": body,
        "session": session,
        "d": d,
    }));
    let v = daemon_call_async("publish", params).await?;
    let title_echo = v["title"].as_str().unwrap_or(&title);
    let d_tag = v["d_tag"].as_str().unwrap_or("?");
    println!("published proposal {} ({})", title_echo, d_tag);
    // The relay accepted the write (or rpc_propose would have errored), but
    // confirm it's actually retrievable. A false here means the relay ACKed then
    // dropped the event — warn loudly so a green publish isn't mistaken for one
    // that landed.
    if v.get("retrievable").is_some() && !v["retrievable"].as_bool().unwrap_or(true) {
        let eid = v["event_id"].as_str().unwrap_or("?");
        eprintln!(
            "{} proposal {} accepted by the relay but NOT retrievable on read-back \
             (event {}). It may not be stored — verify with `tenex-edge debug doctor`.",
            "warning:".yellow(),
            d_tag,
            &eid[..eid.len().min(8)],
        );
    }
    Ok(())
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

// ── envelope rendering ───────────────────────────────────────────────────────

/// Everything needed to render one inbound message as an email-like envelope.
#[cfg(test)]
pub(super) struct EnvelopeView<'a> {
    pub from_slug: &'a str,
    /// Sender's host label. Empty, or equal to `self_host`, → no remote annotation.
    pub host: &'a str,
    /// The viewer's own host, to decide whether the sender is `[remote: …]`.
    pub self_host: &'a str,
    pub subject: &'a str,
    pub branch: &'a str,
    pub commit: &'a str,
    pub dirty: u32,
    /// Short reply id (see `event_short_id`).
    pub id: &'a str,
    /// When the sender published (unix secs); rendered absolute + relative.
    pub sent_at: u64,
    pub now: u64,
    pub body: &'a str,
}

/// Render an inbound message as an email-like envelope:
///
/// ```text
/// From: codex@tenex-edge
/// Date: 2026-06-12 14:23 (3 min ago)
/// Subject: NIP-29 group creation failing
/// Branch: features/oauth (a1b2c3d) [1 file dirty]
/// ID: 01234567
/// --
/// <body>
/// ```
///
/// The Subject and Branch lines are omitted when absent; a remote sender adds
/// `[remote: <host>]` to the From line.
#[cfg(test)]
pub(super) fn format_envelope(e: &EnvelopeView) -> String {
    use crate::util::dirty_label;
    use std::fmt::Write as _;

    // Canonical sender identity is the public agent label with host.
    let from = crate::idref::agent_ref_from(e.from_slug, e.host, e.self_host);

    let mut s = String::new();
    let _ = write!(s, "From: {from}");
    let _ = write!(
        s,
        "\nDate: {} ({})",
        format_local_datetime(e.sent_at),
        crate::util::relative_time(e.sent_at, e.now)
    );
    if !e.subject.is_empty() {
        let _ = write!(s, "\nSubject: {}", e.subject);
    }
    if !e.branch.is_empty() {
        let commit = if e.commit.is_empty() {
            String::new()
        } else {
            format!(" ({})", e.commit)
        };
        let _ = write!(
            s,
            "\nBranch: {}{}{}",
            e.branch,
            commit,
            dirty_label(e.dirty)
        );
    }
    let _ = write!(s, "\nID: {}", e.id);
    let _ = write!(s, "\n--\n{}", e.body);
    s
}

#[cfg(test)]
mod tests;

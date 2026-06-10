use super::*;

// ── send-message ─────────────────────────────────────────────────────────────

pub(super) async fn send_message(
    recipient: String,
    message: String,
    session: Option<String>,
) -> Result<()> {
    let params = serde_json::json!({
        "recipient": recipient,
        "message": message,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("send_message", params).await?;
    let to_pubkey = v["to_pubkey"].as_str().unwrap_or_default().to_string();
    let target_session = v["target_session"].as_str().map(str::to_string);
    match target_session {
        Some(s) => println!(
            "mentioned {} (session {})",
            pubkey_short(&to_pubkey),
            SessionId::from(s)
        ),
        None => println!("mentioned {}", pubkey_short(&to_pubkey)),
    }
    Ok(())
}

/// Async daemon call helper for `async fn` verbs (uses the async client; we are
/// inside the tokio runtime so we must NOT block_on a sync client here).
async fn daemon_call_async(method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}

pub(super) fn resolve_send_message_body(raw: Option<String>) -> Result<String> {
    match raw {
        Some(message) if message == "-" => read_stdin_message(),
        Some(message) if message.is_empty() => bail!("message must not be empty"),
        Some(message) => Ok(message),
        None => {
            if io::stdin().is_terminal() {
                bail!(
                    "missing message; use `tenex-edge send-message --recipient <target> --message \"...\"` \
                     or pipe/heredoc the message on stdin"
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

// ── mention rendering (one place; reused by inbox / wait / turn injection) ────

/// The fully-qualified `--recipient` handle the receiver should reply to. Prefer
/// the sender's exact session id — so a reply reaches the precise sibling session
/// that wrote this — but only when that session actually resolves on our side;
/// otherwise fall back to `slug@project`, which always routes to the agent.
pub fn mention_reply_handle(store: &Store, row: &crate::state::InboxRow) -> String {
    if !row.from_session.is_empty() {
        let resolves = store
            .find_peer_session_by_prefix(&row.from_session)
            .ok()
            .flatten()
            .is_some()
            || store
                .find_session_by_prefix(&row.from_session)
                .ok()
                .flatten()
                .is_some();
        if resolves {
            return row.from_session.clone();
        }
    }
    format!("{}@{}", row.from_slug, row.project)
}

/// One injected line for an inbound mention. `reply_to` is the literal value to
/// pass to `tenex-edge send-message --recipient <reply_to>`.
pub fn format_mention_line(from_slug: &str, project: &str, reply_to: &str, body: &str) -> String {
    format!("[mention from {from_slug}@{project} · reply-to {reply_to}] {body}")
}

// ── inbox ────────────────────────────────────────────────────────────────────

pub(super) async fn inbox(session: Option<String>) -> Result<()> {
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("inbox", params).await?;
    if let Some(rows) = v["rows"].as_array() {
        for r in rows {
            println!(
                "{}",
                format_mention_line(
                    r["from_slug"].as_str().unwrap_or(""),
                    r["project"].as_str().unwrap_or(""),
                    r["reply_to"].as_str().unwrap_or(""),
                    r["body"].as_str().unwrap_or(""),
                )
            );
        }
    }
    if let Some(pending) = v["pending_agents"].as_array().filter(|p| !p.is_empty()) {
        let names: Vec<String> = pending
            .iter()
            .map(|p| {
                format!(
                    "{} ({})",
                    p["slug"].as_str().unwrap_or(""),
                    pubkey_short(p["pubkey"].as_str().unwrap_or(""))
                )
            })
            .collect();
        println!(
            "[tenex-edge] {} unauthorized agent(s) claim your owner: {}. \
They are NOT visible until you decide — tell your human to run `tenex-edge acl` to allow or block them.",
            pending.len(),
            names.join(", ")
        );
    }
    Ok(())
}

// ── wait-for-mention ─────────────────────────────────────────────────────────

pub(super) async fn wait_for_mention(session: Option<String>, timeout: u64) -> Result<()> {
    // The daemon long-polls: it holds the request open until a mention for this
    // session arrives or the timeout fires, then returns the rows.
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        "timeout": timeout,
    });
    let v = daemon_call_async("wait_for_mention", params).await?;
    if let Some(rows) = v["rows"].as_array().filter(|r| !r.is_empty()) {
        for r in rows {
            println!(
                "{}",
                format_mention_line(
                    r["from_slug"].as_str().unwrap_or(""),
                    r["project"].as_str().unwrap_or(""),
                    r["reply_to"].as_str().unwrap_or(""),
                    r["body"].as_str().unwrap_or(""),
                )
            );
        }
        println!("[tenex-edge] Run `tenex-edge wait-for-mention` with run_in_background=true to receive the next mention.");
    }
    Ok(())
}

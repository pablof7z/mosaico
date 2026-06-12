use super::*;

// ── inbox send ───────────────────────────────────────────────────────────────

pub(super) async fn inbox_send(
    recipient: String,
    subject: Option<String>,
    message: String,
    session: Option<String>,
) -> Result<()> {
    let params = serde_json::json!({
        "recipient": recipient,
        "subject": subject,
        "message": message,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("send_message", params).await?;
    print_send_ack(&v);
    Ok(())
}

// ── inbox reply (by ID) ──────────────────────────────────────────────────────

pub(super) async fn inbox_reply(
    id: String,
    subject: Option<String>,
    message: String,
    session: Option<String>,
) -> Result<()> {
    let params = serde_json::json!({
        "id": id,
        "subject": subject,
        "message": message,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("inbox_reply", params).await?;
    print_send_ack(&v);
    Ok(())
}

fn print_send_ack(v: &serde_json::Value) {
    let to_pubkey = v["to_pubkey"].as_str().unwrap_or_default().to_string();
    let target_session = v["target_session"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    match target_session {
        Some(s) => println!(
            "mentioned {} (session {})",
            pubkey_short(&to_pubkey),
            SessionId::from(s)
        ),
        None => println!("mentioned {}", pubkey_short(&to_pubkey)),
    }
}

// ── propose ──────────────────────────────────────────────────────────────────

pub(super) async fn propose(
    title: String,
    body: String,
    thread_id: Option<String>,
    d: Option<String>,
    session: Option<String>,
) -> Result<()> {
    let params = serde_json::json!({
        "title": title,
        "body": body,
        "thread_id": thread_id,
        "d": d,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("propose", params).await?;
    let title_echo = v["title"].as_str().unwrap_or(&title);
    let d_tag = v["d_tag"].as_str().unwrap_or("?");
    println!("published proposal {title_echo} ({d_tag})");
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

// ── envelope rendering (one place; reused by inbox / wait / turn injection) ───

/// The short `ID` shown on an envelope — the first 8 hex chars of the mention's
/// event id. The receiver passes it to `tenex-edge inbox reply --id <ID>`, which
/// matches it back to the full event by prefix.
pub fn mention_short_id(event_id: &str) -> String {
    event_id.chars().take(8).collect()
}

/// Everything needed to render one inbound message as an email-like envelope.
/// Built either daemon-side from an `InboxRow` (turn injection) or client-side
/// from the daemon's JSON (the `inbox` / `wait-for-mention` commands).
pub struct EnvelopeView<'a> {
    pub from_slug: &'a str,
    pub project: &'a str,
    /// Sender's session id (raw; rendered as a stable short code). Empty → omitted.
    pub from_session: &'a str,
    /// Sender's host label. Empty, or equal to `self_host`, → no remote annotation.
    pub host: &'a str,
    /// The viewer's own host, to decide whether the sender is `[remote: …]`.
    pub self_host: &'a str,
    pub subject: &'a str,
    pub branch: &'a str,
    pub commit: &'a str,
    pub dirty: u32,
    /// Short reply id (see `mention_short_id`).
    pub id: &'a str,
    /// When the sender published (unix secs); rendered absolute + relative.
    pub sent_at: u64,
    pub now: u64,
    pub body: &'a str,
}

/// Render an inbound message as an email-like envelope:
///
/// ```text
/// From: codex@tenex-edge [session ca0ff4]
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
pub fn format_envelope(e: &EnvelopeView) -> String {
    let mut from = format!("{}@{}", e.from_slug, e.project);
    if !e.from_session.is_empty() {
        let _ = write!(from, " [session {}]", session_short_code(e.from_session));
    }
    if !e.host.is_empty() && slugify_host(e.host) != slugify_host(e.self_host) {
        let _ = write!(from, " [remote: {}]", e.host);
    }

    let mut s = String::new();
    let _ = write!(s, "From: {from}");
    let _ = write!(
        s,
        "\nDate: {} ({})",
        format_local_datetime(e.sent_at),
        relative_time(e.sent_at, e.now)
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
        let _ = write!(s, "\nBranch: {}{}{}", e.branch, commit, dirty_label(e.dirty));
    }
    let _ = write!(s, "\nID: {}", e.id);
    let _ = write!(s, "\n--\n{}", e.body);
    s
}

/// Render a `serde_json` row (as produced by the daemon's `rows_to_json`) into an
/// envelope. Used by the `inbox` and `wait-for-mention` CLI commands.
fn format_envelope_json(r: &serde_json::Value, now: u64) -> String {
    format_envelope(&EnvelopeView {
        from_slug: r["from_slug"].as_str().unwrap_or(""),
        project: r["project"].as_str().unwrap_or(""),
        from_session: r["from_session"].as_str().unwrap_or(""),
        host: r["host"].as_str().unwrap_or(""),
        self_host: r["self_host"].as_str().unwrap_or(""),
        subject: r["subject"].as_str().unwrap_or(""),
        branch: r["branch"].as_str().unwrap_or(""),
        commit: r["commit"].as_str().unwrap_or(""),
        dirty: r["dirty"].as_u64().unwrap_or(0) as u32,
        id: r["id"].as_str().unwrap_or(""),
        sent_at: r["created_at"].as_u64().unwrap_or(0),
        now,
        body: r["body"].as_str().unwrap_or(""),
    })
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
        let now = now_secs();
        for (i, r) in rows.iter().enumerate() {
            if i > 0 {
                println!();
            }
            println!("{}", format_envelope_json(r, now));
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
        let now = now_secs();
        for (i, r) in rows.iter().enumerate() {
            if i > 0 {
                println!();
            }
            println!("{}", format_envelope_json(r, now));
        }
        println!("\n[tenex-edge] Run `tenex-edge wait-for-mention` with run_in_background=true to receive the next mention.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>() -> EnvelopeView<'a> {
        EnvelopeView {
            from_slug: "codex",
            project: "tenex-edge",
            from_session: "sender-session-id",
            host: "",
            self_host: "my-box",
            subject: "NIP-29 group creation failing",
            branch: "features/oauth",
            commit: "a1b2c3d",
            dirty: 0,
            id: "01234567",
            sent_at: 1_000,
            now: 1_180, // +3 min
            body: "can you take a look?",
        }
    }

    #[test]
    fn envelope_has_email_like_headers_then_body() {
        let out = format_envelope(&view());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            format!(
                "From: codex@tenex-edge [session {}]",
                session_short_code("sender-session-id")
            )
        );
        assert!(lines[1].starts_with("Date: ") && lines[1].ends_with("(3 min ago)"));
        assert_eq!(lines[2], "Subject: NIP-29 group creation failing");
        assert_eq!(lines[3], "Branch: features/oauth (a1b2c3d)");
        assert_eq!(lines[4], "ID: 01234567");
        assert_eq!(lines[5], "--");
        assert_eq!(lines[6], "can you take a look?");
    }

    #[test]
    fn dirty_count_and_remote_host_annotate() {
        let mut v = view();
        v.dirty = 1;
        v.host = "prod-01.example.com";
        let out = format_envelope(&v);
        assert!(out.contains("[remote: prod-01.example.com]"));
        assert!(out.contains("Branch: features/oauth (a1b2c3d) [1 file dirty]"));
        v.dirty = 3;
        assert!(format_envelope(&v).contains("[3 files dirty]"));
    }

    #[test]
    fn subject_and_branch_lines_omitted_when_empty() {
        let mut v = view();
        v.subject = "";
        v.branch = "";
        let out = format_envelope(&v);
        assert!(!out.contains("Subject:"));
        assert!(!out.contains("Branch:"));
        // Same-host sender → no remote annotation.
        assert!(!out.contains("remote:"));
    }
}

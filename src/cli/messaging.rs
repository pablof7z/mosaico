use super::*;

// ── send-message ─────────────────────────────────────────────────────────────

/// Where an `inbox send` should go. Mirrors the two mutually-exclusive
/// addressing flags on the CLI (`--to-new-session` / `--to-session`).
pub(super) enum SendTarget {
    /// Spawn a fresh session of `agent` (in `project`, else the cwd's project)
    /// and deliver the message to it.
    NewSession {
        agent: String,
        project: Option<String>,
    },
    /// Deliver to an existing session, addressed by id or codename.
    Session(String),
}

pub(super) async fn inbox_send(
    target: SendTarget,
    subject: Option<String>,
    message: String,
    session: Option<String>,
    thread_id: Option<String>,
) -> Result<()> {
    // `recipient` carries the agent slug or session handle; `mode` tells the
    // daemon which one and whether to spawn. `project` only applies to
    // new-session sends.
    let (recipient, mode, project) = match target {
        SendTarget::NewSession { agent, project } => (agent, "new_session", project),
        SendTarget::Session(sess) => (sess, "session", None),
    };
    let params = serde_json::json!({
        "recipient": recipient,
        "mode": mode,
        "project": project,
        "subject": subject,
        "message": message,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": agent_env_slug(),
        "group": crate::cli::group_env(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        "thread_id": thread_id,
    });
    let v = daemon_call_async("send_message", params).await?;
    print_send_ack(&v);
    Ok(())
}

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
        "agent": agent_env_slug(),
        "group": crate::cli::group_env(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("inbox_reply", params).await?;
    print_send_ack(&v);
    Ok(())
}

pub(super) async fn chat_write(
    message: String,
    mention: Option<String>,
    session: Option<String>,
) -> Result<()> {
    let params = serde_json::json!({
        "message": message,
        "mention": mention,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": agent_env_slug(),
        "group": crate::cli::group_env(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("chat_write", params).await?;
    let event_id = v["event_id"].as_str().unwrap_or("?");
    if let Some(session) = v["mentioned_session"].as_str().filter(|s| !s.is_empty()) {
        println!(
            "sent chat {} mentioning session {}",
            pubkey_short(event_id),
            SessionId::from(session.to_string())
        );
    } else {
        println!("sent chat {}", pubkey_short(event_id));
    }
    Ok(())
}

pub(super) async fn chat_read(
    since: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
    tail: bool,
    live: bool,
    project: Option<String>,
) -> Result<()> {
    use std::io::IsTerminal as _;

    let project = project
        .unwrap_or_else(|| crate::project::resolve(&std::env::current_dir().unwrap_or_default()));
    let since_ts = since.as_deref().map(super::admin::parse_since);
    let effective_tail = tail || since.is_none();
    let effective_limit = limit.or_else(|| {
        if since.is_none() || tail {
            Some(10)
        } else {
            None
        }
    });
    let use_color = std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal();

    let params = serde_json::json!({
        "project": project,
        "since": since_ts,
        "limit": effective_limit,
        "offset": offset.unwrap_or(0),
        "tail": effective_tail,
        "live": live,
    });
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client
        .stream("chat_read", params, move |item| {
            println!("{}", render_chat_read_row(&item, use_color));
        })
        .await
}

fn render_chat_read_row(item: &serde_json::Value, use_color: bool) -> String {
    let pubkey = item["from_pubkey"].as_str().unwrap_or_default();
    let slug = item["from_slug"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| pubkey_short(pubkey));
    let host = item["host"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("?");
    let sender = format!("<{slug}@{host}>");
    let sender = color_by_pubkey(&sender, pubkey, use_color);
    let body = item["body"].as_str().unwrap_or_default().trim_end();
    let ts = item["created_at"].as_u64().unwrap_or(0);
    format!("{sender} {body} [{}]", format_local_datetime(ts))
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

// ── propose ───────────────────────────────────────────────────────────────────

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
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": agent_env_slug(),
        "group": crate::cli::group_env(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        "thread_id": thread_id,
        "d": d,
    });
    let v = daemon_call_async("propose", params).await?;
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
             (event {}). It may not be stored — verify with `tenex-edge doctor`.",
            "warning:".yellow(),
            d_tag,
            &eid[..eid.len().min(8)],
        );
    }
    Ok(())
}

// ── threads ───────────────────────────────────────────────────────────────────

/// `threads`: list threads for a project, or messages for a specific thread.
///
/// Routes to the daemon via `list_threads`, `messages`, or `thread_meta` RPCs
/// and prints a human-readable summary.
pub(super) async fn threads(project: Option<String>, thread: Option<String>) -> Result<()> {
    if let Some(tid) = thread {
        // Show messages for a specific thread.
        let v = daemon_call_async("messages", serde_json::json!({ "thread_id": tid })).await?;
        let meta_v =
            daemon_call_async("thread_meta", serde_json::json!({ "thread_id": tid })).await?;

        if let Some(subject) = meta_v.get("subject").and_then(|v| v.as_str()) {
            println!("Thread: {}", subject);
        } else {
            println!("Thread: {}", pubkey_short(&tid));
        }
        if let Some(msgs) = v.as_array() {
            for msg in msgs {
                let dir = msg["direction"].as_str().unwrap_or("?");
                let author = msg["author_pubkey"].as_str().unwrap_or("?");
                let body = msg["body"].as_str().unwrap_or("");
                let ts = msg["created_at"].as_u64().unwrap_or(0);
                let arrow = if dir == "outbound" { "->" } else { "<-" };
                println!(
                    "[{}] {} {} {}: {}",
                    ts,
                    pubkey_short(author),
                    arrow,
                    dir,
                    body
                );
            }
        }
        return Ok(());
    }

    // List threads for a project.
    let proj = project
        .unwrap_or_else(|| crate::project::resolve(&std::env::current_dir().unwrap_or_default()));
    let v = daemon_call_async("list_threads", serde_json::json!({ "project": proj })).await?;
    if let Some(threads) = v.as_array() {
        if threads.is_empty() {
            println!("No threads in project {:?}", proj);
            return Ok(());
        }
        println!("Threads in {}:", proj);
        for t in threads {
            let tid = t["thread_id"].as_str().unwrap_or("?");
            let count = t["message_count"].as_u64().unwrap_or(0);
            let last = t["last_message_at"].as_u64();
            let subject = t["subject"].as_str();
            let label = subject.unwrap_or("no subject");
            match last {
                // Print the FULL thread id — it is the argument the user passes
                // back to `threads --thread <id>`; a pubkey_short() would be unusable.
                Some(ts) => println!("  {} ({} msg, last at {}) - {}", tid, count, ts, label),
                None => println!("  {} (no messages) - {}", tid, label),
            }
        }
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

// ── mention rendering (one place; reused by inbox / turn injection) ───────────

/// The fully-qualified `--recipient` handle the receiver should reply to. Prefer
/// the sender's exact session id — so a reply reaches the precise sibling session
/// that wrote this — but only when that session actually resolves on our side;
/// otherwise fall back to `slug@project`, which always routes to the agent.
/// Render an `InboxRow` as an email-like envelope (the daemon-side path; the CLI
/// path renders from JSON). `self_host` decides the `[remote: …]` annotation.
pub(crate) fn row_envelope(r: &crate::state::InboxRow, self_host: &str, now: u64) -> String {
    let id = mention_short_id(&r.mention_event_id);
    format_envelope(&EnvelopeView {
        from_slug: &r.from_slug,
        project: &r.project,
        from_session: &r.from_session,
        host: &r.host,
        self_host,
        subject: &r.subject,
        branch: &r.branch,
        commit: &r.commit,
        dirty: r.dirty,
        id: &id,
        sent_at: r.created_at,
        now,
        body: &r.body,
    })
}

// ── envelope rendering (one place; reused by inbox / turn injection) ─────────

/// The short `ID` shown on an envelope — the first 8 hex chars of the mention's
/// event id. The receiver passes it to `tenex-edge inbox reply --id <ID>`, which
/// matches it back to the full event by prefix.
pub fn mention_short_id(event_id: &str) -> String {
    event_id.chars().take(8).collect()
}

/// Everything needed to render one inbound message as an email-like envelope.
/// Built either daemon-side from an `InboxRow` (turn injection) or client-side
/// from the daemon's JSON (the `inbox` command).
pub struct EnvelopeView<'a> {
    pub from_slug: &'a str,
    pub project: &'a str,
    /// Sender's session id (raw; rendered as a stable codename). Empty → omitted.
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
        let _ = write!(from, " [session {}]", session_codename(e.from_session));
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

/// Render a `serde_json` row (as produced by the daemon's `rows_to_json`) into an
/// envelope. Used by the `inbox` CLI command.
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
        "agent": agent_env_slug(),
        "group": crate::cli::group_env(),
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
    Ok(())
}

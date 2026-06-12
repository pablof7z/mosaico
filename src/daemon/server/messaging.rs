use super::session::resolve_session;
use super::*;

// ── send_message ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct SendMessageParams {
    recipient: String,
    message: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

pub(super) async fn rpc_send_message(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SendMessageParams =
        serde_json::from_value(params.clone()).context("parsing send_message params")?;
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;

    let recipient = state.with_store(|s| resolve_recipient(s, &rec.project, &p.recipient))?;

    let meta = workspace_meta(state, p.cwd.as_deref(), p.subject.unwrap_or_default(), None);
    let mention = Mention {
        from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
        to_pubkey: recipient.pubkey.clone(),
        project: recipient.project.clone(),
        body: p.message,
        target_session: recipient.target_session.clone().map(SessionId::from),
        // Stamp the sender's own session so the recipient can reply to it precisely.
        from_session: Some(SessionId::from(rec.session_id.clone())),
        meta,
    };
    let builder = state.codec.encode(&DomainEvent::Mention(mention.clone()))?;
    // Publish over the shared relay; the returned EventId is the canonical id of
    // the just-signed event.
    let event_id = state.transport.publish_signed(builder, &id.keys).await?;

    // LOCAL DELIVERY (the same-machine fix). When the recipient is an agent this
    // daemon hosts (e.g. a SIBLING claude session sharing the sender's pubkey),
    // delivery must NOT depend on the relay echoing our own published event back
    // to us — relays generally do not re-deliver an event to the connection that
    // published it. Route the mention into the recipient's session inbox(es) here,
    // keyed by the SAME EventId we just published. `route_mention_into` →
    // `enqueue_mention` is idempotent on `(mention_event_id, target_session)`, so
    // if the relay does echo it later, no duplicate is created. `compute_targets`
    // delivers only to the TARGET session (or all of the recipient agent's
    // sessions when untargeted) — never back to the authoring session.
    if state
        .hosted_pubkeys()
        .iter()
        .any(|h| h == &recipient.pubkey)
    {
        let routed = state.with_store(|s| {
            route_mention_into_with_id(
                s,
                &recipient.pubkey,
                &mention,
                &event_id.to_hex(),
                now_secs(),
            )
        });
        if routed {
            state.mention_notify.notify_waiters();
        }
    }

    Ok(
        serde_json::json!({ "to_pubkey": recipient.pubkey, "target_session": recipient.target_session }),
    )
}

// ── inbox reply (reply by mention ID) ─────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct InboxReplyParams {
    /// Short `ID` from an envelope (prefix of the original mention's event id).
    id: String,
    message: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

/// Reply to a mention by its short `ID`. Looks up the original inbox row, then
/// publishes a kind:1 that `p`-tags the original sender and `e`-tags (NIP-10
/// reply) the original event — threading the reply back to exactly the sender
/// session that wrote it. Subject defaults to `Re: <original subject>`.
pub(super) async fn rpc_inbox_reply(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: InboxReplyParams =
        serde_json::from_value(params.clone()).context("parsing inbox_reply params")?;
    if p.id.is_empty() {
        anyhow::bail!("missing --id (the ID shown on the message you're replying to)");
    }
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;

    let original = state
        .with_store(|s| s.find_inbox_by_event_prefix(&p.id))?
        .with_context(|| format!("no message in this inbox with ID {:?}", p.id))?;

    // Default the subject to `Re: <original>` (don't double-prefix on a reply chain).
    let subject = match p.subject {
        Some(s) if !s.is_empty() => s,
        _ if original.subject.is_empty() => String::new(),
        _ if original.subject.to_lowercase().starts_with("re:") => original.subject.clone(),
        _ => format!("Re: {}", original.subject),
    };

    let mut meta = workspace_meta(state, p.cwd.as_deref(), subject, None);
    meta.reply_to_event_id = Some(original.mention_event_id.clone());

    let mention = Mention {
        from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
        to_pubkey: original.from_pubkey.clone(),
        project: original.project.clone(),
        body: p.message,
        // Route back to the precise sender session when we captured one.
        target_session: Some(original.from_session.clone())
            .filter(|s| !s.is_empty())
            .map(SessionId::from),
        from_session: Some(SessionId::from(rec.session_id.clone())),
        meta,
    };
    let builder = state.codec.encode(&DomainEvent::Mention(mention.clone()))?;
    let event_id = state.transport.publish_signed(builder, &id.keys).await?;

    // Local delivery to a same-machine sibling session (see rpc_send_message).
    if state.hosted_pubkeys().iter().any(|h| h == &original.from_pubkey) {
        let routed = state.with_store(|s| {
            route_mention_into_with_id(
                s,
                &original.from_pubkey,
                &mention,
                &event_id.to_hex(),
                now_secs(),
            )
        });
        if routed {
            state.mention_notify.notify_waiters();
        }
    }

    Ok(serde_json::json!({
        "to_pubkey": original.from_pubkey,
        "target_session": original.from_session,
        "in_reply_to": original.mention_event_id,
    }))
}

/// Capture the sender's envelope metadata: `subject` plus a snapshot of the git
/// workspace at `cwd` (branch, short commit, dirty-file count) and this daemon's
/// host. `reply_to` is left `None` here; callers set it for replies.
fn workspace_meta(
    state: &Arc<DaemonState>,
    cwd: Option<&str>,
    subject: String,
    reply_to: Option<String>,
) -> crate::domain::MentionMeta {
    let (branch, commit, dirty) = git_snapshot(cwd);
    crate::domain::MentionMeta {
        subject,
        branch,
        commit,
        dirty,
        host: state.host.clone(),
        reply_to_event_id: reply_to,
    }
}

/// `(branch, short_commit, dirty_count)` for the git repo at `cwd` (or the
/// daemon's cwd when `None`). All-empty / zero when `cwd` isn't a git repo.
/// `dirty_count` is `git status --porcelain` line count, which already excludes
/// gitignored files.
fn git_snapshot(cwd: Option<&str>) -> (String, String, u32) {
    use std::process::Command;
    let dir = cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").arg("-C").arg(&dir).args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let dirty = git(&["status", "--porcelain"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as u32)
        .unwrap_or(0);
    (branch, commit, dirty)
}

// ── propose ──────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct ProposeParams {
    title: String,
    body: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    /// Event id of the conversation this proposal belongs to (NIP-10 "e" root tag).
    #[serde(default)]
    thread_id: Option<String>,
    /// Stable `d` tag identifier. Supply to revise an existing proposal at the
    /// same (author, d) naddr; omit to mint a fresh one.
    #[serde(default)]
    d: Option<String>,
}

/// Publish a kind:30023 (NIP-23 long-form) proposal signed by the agent's identity.
///
/// Tags:
///   ["d", <id>]                    — NIP-33 addressable identifier
///   ["title", <title>]             — human-readable title
///   ["h", <project>]               — NIP-29 group
///   ["p", <owner>]…                — one per owner; surfaces to the human
///   ["e", <thread_id>, "", "root"] — when --thread given; links to conversation
///   ["session-id", <session>]      — authoring session (omitted when no live session)
pub(super) async fn rpc_propose(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::{EventBuilder, Kind, Tag};

    let p: ProposeParams =
        serde_json::from_value(params.clone()).context("parsing propose params")?;
    if p.title.is_empty() {
        anyhow::bail!("title must not be empty");
    }

    // Resolve session if one is live; fall back to cwd-based project + env agent.
    // propose doesn't require a live session — it just needs a project and a key.
    let session_rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )
    .ok();

    let cwd = p
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project = session_rec
        .as_ref()
        .map(|r| r.project.clone())
        .unwrap_or_else(|| crate::project::resolve(&cwd));
    let agent_slug = session_rec
        .as_ref()
        .map(|r| r.agent_slug.clone())
        .or_else(|| p.agent.clone().filter(|a| !a.is_empty()))
        .unwrap_or_else(|| "agent".to_string());

    let id = identity::load_or_create(&config::edge_home(), &agent_slug, now_secs())?;

    let d_tag =
        p.d.clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("prop-{:x}", now_secs()));

    let mut tags: Vec<Tag> = vec![
        Tag::parse(["d", &d_tag]).context("d tag")?,
        Tag::parse(["title", &p.title]).context("title tag")?,
        Tag::parse(["h", &project]).context("h tag")?,
    ];
    if let Some(ref rec) = session_rec {
        tags.push(Tag::parse(["session-id", rec.session_id.as_str()]).context("session-id tag")?);
    }
    for owner in &state.owners {
        tags.push(Tag::parse(["p", owner]).context("p tag")?);
    }
    if let Some(ref eid) = p.thread_id {
        tags.push(Tag::parse(["e", eid, "", "root"]).context("e root tag")?);
    }

    let builder = EventBuilder::new(Kind::from(30023u16), p.body).tags(tags);
    let event_id = state
        .transport
        .publish_signed(builder, &id.keys)
        .await
        .context("publishing proposal")?;

    Ok(serde_json::json!({
        "event_id": event_id.to_hex(),
        "d_tag": d_tag,
        "title": p.title,
    }))
}

struct ResolvedRecipient {
    pubkey: String,
    target_session: Option<String>,
    project: String,
}

fn resolve_recipient(store: &Store, my_project: &str, target: &str) -> Result<ResolvedRecipient> {
    if let Some((slug, proj)) = target.split_once('@') {
        let pk = store
            .resolve_agent_pubkey(slug, Some(proj))?
            .with_context(|| {
                format!("can't resolve {slug}@{proj} (no presence/profile seen yet)")
            })?;
        return Ok(ResolvedRecipient {
            pubkey: pk,
            target_session: None,
            project: proj.to_string(),
        });
    }
    if target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(ResolvedRecipient {
            pubkey: target.to_string(),
            target_session: None,
            project: my_project.to_string(),
        });
    }
    if target.len() >= 6 {
        if let Some(ps) = store.find_peer_session_by_prefix(target)? {
            return Ok(ResolvedRecipient {
                pubkey: ps.pubkey,
                target_session: Some(ps.session_id),
                project: ps.project,
            });
        }
        if let Some(s) = store.find_session_by_prefix(target)? {
            return Ok(ResolvedRecipient {
                pubkey: s.agent_pubkey,
                target_session: Some(s.session_id),
                project: s.project,
            });
        }
        // Try matching against hash-based session short codes (from `who` display).
        // This is a fallback for when users copy session codes from `who` output.
        if let Some(found) = find_session_by_hash(store, target)? {
            return Ok(ResolvedRecipient {
                pubkey: found.pubkey,
                target_session: Some(found.session_id),
                project: found.project,
            });
        }
    }
    if let Some(pk) = store.resolve_agent_pubkey(target, Some(my_project))? {
        return Ok(ResolvedRecipient {
            pubkey: pk,
            target_session: None,
            project: my_project.to_string(),
        });
    }
    anyhow::bail!("can't resolve recipient {target:?} (try `tenex-edge who`)")
}

struct SessionMatch {
    pubkey: String,
    session_id: String,
    project: String,
}

/// Try to find a session (peer or own) matching the given hash code.
/// Hash codes are what `who` displays for sessions (6-char hex strings).
fn find_session_by_hash(store: &Store, hash_code: &str) -> Result<Option<SessionMatch>> {
    let target_code = hash_code.to_lowercase();

    // Search peer sessions
    if let Ok(peers) = store.list_peer_sessions(None, 0) {
        for peer in peers {
            if session_short_code(&peer.session_id).to_lowercase() == target_code {
                return Ok(Some(SessionMatch {
                    pubkey: peer.pubkey,
                    session_id: peer.session_id,
                    project: peer.project,
                }));
            }
        }
    }

    // Search own sessions
    if let Ok(sessions) = store.list_my_live_sessions(0) {
        for session in sessions {
            if session_short_code(&session.session_id).to_lowercase() == target_code {
                return Ok(Some(SessionMatch {
                    pubkey: session.agent_pubkey,
                    session_id: session.session_id,
                    project: session.project,
                }));
            }
        }
    }

    Ok(None)
}

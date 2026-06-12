use super::session::resolve_session;
use super::*;

#[derive(serde::Deserialize, Default)]
struct InboxParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

pub(super) async fn rpc_inbox(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: InboxParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let _ = fetch_mentions_into_inbox(state, &rec).await;

    let rows = state.with_store(|s| {
        let rows = s.drain_inbox(&rec.session_id).unwrap_or_default();
        for r in &rows {
            s.mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                .ok();
        }
        rows
    });
    let pending = state.with_store(|s| s.list_pending_agents().unwrap_or_default());
    let rows_json = state.with_store(|s| rows_to_json(s, &rows));

    Ok(serde_json::json!({
        "rows": rows_json,
        "pending_agents": pending.iter().map(|p| serde_json::json!({"slug": p.slug, "pubkey": p.pubkey})).collect::<Vec<_>>(),
    }))
}

#[derive(serde::Deserialize, Default)]
struct TurnStartParams {
    session: String,
    #[serde(default)]
    transcript: Option<String>,
}

pub(super) async fn rpc_turn_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnStartParams =
        serde_json::from_value(params.clone()).context("parsing turn_start params")?;
    if p.session.is_empty() {
        return Ok(serde_json::json!({ "context": serde_json::Value::Null }));
    }

    let prev_started = state.with_store(|s| {
        let (_, prev) = s.get_turn_state(&p.session).unwrap_or((false, 0));
        s.mark_turn_start(&p.session, now_secs()).ok();
        if let Some(path) = p.transcript.as_deref().filter(|x| !x.is_empty()) {
            s.set_session_transcript(&p.session, path).ok();
            // Snapshot the last assistant text so rpc_turn_end can poll until a
            // *new* (different) response appears — Claude Code writes the
            // transcript after the stop hook fires, so reading at stop time often
            // returns the previous turn's content.
            let baseline = crate::transcript::read_last_assistant_text(std::path::Path::new(path))
                .unwrap_or_default();
            s.set_last_assistant_text_at_turn_start(&p.session, &baseline)
                .ok();
        }
        prev
    });

    let rec = match state.with_store(|s| s.get_session(&p.session).ok().flatten()) {
        Some(r) => r,
        None => return Ok(serde_json::json!({ "context": serde_json::Value::Null })),
    };
    // Self-fetch stored mentions (relay), then assemble via the SHARED cli.rs
    // function so the injected text is byte-identical to the pre-daemon CLI and
    // cannot drift.
    let _ = fetch_mentions_into_inbox(state, &rec).await;
    let context = crate::cli::assemble_turn_start_context(&state.store, &rec, prev_started)
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context }))
}

#[derive(serde::Deserialize, Default)]
struct TurnCheckParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

pub(super) fn rpc_turn_check(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnCheckParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let context = crate::cli::assemble_turn_check_context(&state.store, &rec.session_id)
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context }))
}

#[derive(serde::Deserialize)]
struct TurnEndParams {
    session: String,
}

pub(super) async fn rpc_turn_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use crate::domain::{AgentRef, DomainEvent, TurnReply};
    use std::path::Path;

    let p: TurnEndParams =
        serde_json::from_value(params.clone()).context("parsing turn_end params")?;
    if p.session.is_empty() {
        return Ok(serde_json::json!({ "ok": true }));
    }

    // Collect everything we need while holding the store lock, then release it.
    // The IDs are captured NOW so a concurrent user_prompt for the next turn
    // cannot overwrite last_prompt_event_id before we publish.
    let (root_event_id, last_prompt_event_id, transcript_path, baseline_text, session_rec) = state
        .with_store(|s| {
            s.mark_turn_end(&p.session).ok();
            let (root, prompt) = s.get_thread_event_ids(&p.session);
            let transcript = s.get_session_transcript(&p.session).ok().flatten();
            let baseline = s.get_last_assistant_text_at_turn_start(&p.session);
            let rec = s.get_session(&p.session).ok().flatten();
            (root, prompt, transcript, baseline, rec)
        });

    // Publish the TurnReply when we have full threading context.
    if !root_event_id.is_empty() && !last_prompt_event_id.is_empty() {
        if let Some(rec) = session_rec {
            // Claude Code writes the transcript *after* the stop hook fires, so
            // the response may not be on disk yet. Poll (up to ~2 s) until the
            // last assistant text differs from what we snapshotted at turn_start.
            let body = if let Some(path) = transcript_path.as_deref() {
                let mut result = String::new();
                for _ in 0..20u8 {
                    if let Some(text) = crate::transcript::read_last_assistant_text(Path::new(path))
                    {
                        if !text.is_empty() && text != baseline_text {
                            result = text;
                            break;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                result
            } else {
                String::new()
            };

            if !body.is_empty() {
                let ev = DomainEvent::TurnReply(TurnReply {
                    agent: AgentRef::new(rec.agent_pubkey.clone(), rec.agent_slug.clone()),
                    project: rec.project.clone(),
                    body,
                    root_event_id,
                    reply_event_id: last_prompt_event_id,
                });
                let edge = crate::config::edge_home();
                if let Ok(id) =
                    crate::identity::load_or_create(&edge, &rec.agent_slug, crate::util::now_secs())
                {
                    if let Ok(builder) = state.codec.encode(&ev) {
                        if let Ok(reply_eid) =
                            state.transport.publish_signed(builder, &id.keys).await
                        {
                            let sid = p.session.clone();
                            let eid_hex = reply_eid.to_hex();
                            state.with_store(|s| {
                                s.set_last_agent_reply_event_id(&sid, &eid_hex).ok();
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(serde_json::json!({ "ok": true }))
}

// ── user_prompt ──────────────────────────────────────────────────────────────

/// Publish a kind:1 OP signed by the human user's nsec. The event records the
/// user's prompt on the Nostr fabric as a root note (no `e` tag) in the NIP-29
/// group, p-tagging the agent that will process it.
pub(super) async fn rpc_user_prompt(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

    #[derive(serde::Deserialize, Default)]
    struct P {
        #[serde(default)]
        session: Option<String>,
        #[serde(default)]
        env_session: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        agent: Option<String>,
    }
    let p: P = serde_json::from_value(params.clone()).unwrap_or_default();

    let nsec = match &state.cfg.user_nsec {
        Some(n) => n.clone(),
        None => anyhow::bail!("userNsec not set in ~/.tenex/config.json"),
    };
    let user_keys = Keys::parse(&nsec).context("parsing userNsec")?;

    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let body = p.prompt.unwrap_or_default();

    // Read NIP-10 thread state before publishing so we can tag this prompt.
    let (thread_root, last_agent_reply) = state.with_store(|s| {
        let (root, _) = s.get_thread_event_ids(&rec.session_id);
        let agent_reply = s.get_last_agent_reply_event_id(&rec.session_id);
        (root, agent_reply)
    });

    // Include `session-id` so the event is session-scoped on the wire.  Without
    // it, any inbox-routing path (live subscription or fetch) would fan out this
    // event to ALL sessions of the agent, leaking one session's user prompt into
    // a sibling session's inbox.
    //
    // For non-root prompts also carry NIP-10 threading so clients can display
    // the full conversation thread. The first prompt is the root (no e-tags);
    // subsequent prompts reply to the last agent TurnReply.
    let builder = if !thread_root.is_empty() && !last_agent_reply.is_empty() {
        EventBuilder::new(Kind::from(1u16), body).tags([
            Tag::parse(["h", &rec.project])?,
            Tag::parse(["p", &rec.agent_pubkey])?,
            Tag::parse(["session-id", &rec.session_id])?,
            Tag::parse(["e", &thread_root, "", "root"])?,
            Tag::parse(["e", &last_agent_reply, "", "reply"])?,
        ])
    } else {
        EventBuilder::new(Kind::from(1u16), body).tags([
            Tag::parse(["h", &rec.project])?,
            Tag::parse(["p", &rec.agent_pubkey])?,
            Tag::parse(["session-id", &rec.session_id])?,
        ])
    };
    let event_id = state.transport.publish_signed(builder, &user_keys).await?;

    let eid = event_id.to_hex();
    let sid = rec.session_id.clone();
    state.with_store(|s| {
        // Pre-mark as consumed (belt-and-suspenders; owners check is primary guard).
        s.suppress_inbox_event(&sid, &eid).ok();
        // NIP-10 thread tracking: first prompt becomes the root; every prompt is
        // the "last trigger" the next TurnReply will reply to.
        let (root, _) = s.get_thread_event_ids(&sid);
        let new_root = if root.is_empty() { eid.clone() } else { root };
        s.set_thread_event_ids(&sid, &new_root, &eid).ok();
    });

    Ok(serde_json::json!({ "event_id": eid }))
}

// ── startup fetch of stored mentions (offline delivery) ──────────────────────

pub(super) async fn fetch_mentions_into_inbox(
    state: &Arc<DaemonState>,
    rec: &crate::state::SessionRecord,
) -> Result<()> {
    use nostr_sdk::prelude::{Filter, Kind, PublicKey};
    let me = rec.agent_pubkey.clone();
    let pk = PublicKey::from_hex(&me)?;
    let filter = Filter::new().kind(Kind::from(1u16)).pubkey(pk).limit(50);
    if let Ok(events) = state.transport.fetch(filter, Duration::from_secs(3)).await {
        for ev in events {
            // User-prompt events are operator-signed (same key as userNsec owners).
            // They are not inter-agent Mentions; skip them here.
            if state.owners.contains(&ev.pubkey.to_hex()) {
                continue;
            }
            if let Some(DomainEvent::Mention(m)) = state.codec.decode(&ev) {
                if m.to_pubkey != me {
                    continue;
                }
                let to = me.clone();
                let routed = state.with_store(|s| route_mention_into(s, &to, &m, &ev));
                if routed {
                    state.mention_notify.notify_waiters();
                }
            }
        }
    }
    Ok(())
}

pub(super) fn rows_to_json(store: &Store, rows: &[InboxRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|r| {
            serde_json::json!({
                "from_slug": r.from_slug,
                "project": r.project,
                "body": r.body,
                "mention_event_id": r.mention_event_id,
                "from_session": r.from_session,
                // Fully-qualified handle the receiver passes to `--recipient`.
                "reply_to": crate::cli::mention_reply_handle(store, r),
            })
        })
        .collect()
}

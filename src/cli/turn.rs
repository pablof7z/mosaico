use super::messaging::{format_envelope, mention_short_id, EnvelopeView};
use crate::state::InboxRow;
use super::who::push_turn_fabric_block;
use super::*;

// ── turn-start / turn-check / turn-end ───────────────────────────────────────

pub(super) async fn turn_start(
    session: String,
    transcript: Option<String>,
    json_out: bool,
) -> Result<()> {
    if session.is_empty() {
        return Ok(());
    }
    let params = serde_json::json!({
        "session": session,
        "transcript": transcript,
    });
    let v = daemon_call_async("turn_start", params).await?;
    if let Some(ctx) = v["context"].as_str() {
        emit_context(ctx, json_out);
    }
    Ok(())
}

/// The full turn-start context assembly, shared by the daemon's `turn_start` RPC
/// (the only caller now). Mutating reads (mark_turn_start, drain, set_transcript)
/// happen here under the shared store; the relay self-fetch is done by the
/// caller beforehand. Single source of truth → injected text cannot drift.
///
/// `prev_turn_started_at` is the turn_state value BEFORE this turn's mark; the
/// caller passes it so first-turn detection matches the old behavior.
pub fn assemble_turn_start_context(
    store: &std::sync::Mutex<Store>,
    rec: &crate::state::SessionRecord,
    prev_turn_started_at: u64,
) -> Option<String> {
    let first_turn = prev_turn_started_at == 0;
    let mut blocks: Vec<String> = Vec::new();

    if first_turn {
        blocks.push(
            "[tenex-edge] You are connected to the tenex-edge agent fabric. \
             You can run `tenex-edge who`, `tenex-edge inbox`, and \
             `tenex-edge inbox send --to <agent@project|session-id> --subject \"...\" --message \"...\"`. \
             Reply to a message you received with `tenex-edge inbox reply --id <ID> \"...\"`. \
             If the user asks you to message/contact/tell another agent, run `tenex-edge inbox send`; \
             do not say you cannot send messages from here. Run `tenex-edge wait-for-mention` \
             with run_in_background=true so you are woken when a mention arrives. \
             Re-run it each time one is received."
                .to_string(),
        );

        // Warn if this agent couldn't be added to the NIP-29 group (e.g. the
        // daemon on this machine is not the relay admin). The session-start hook
        // tried and failed silently; surface it here so the agent can tell the
        // user what to fix.
        let not_member = {
            let s = store.lock().expect("store mutex poisoned");
            !s.is_group_member(&rec.project, &rec.agent_pubkey)
                .unwrap_or(true)
        };
        if not_member {
            blocks.push(format!(
                "[tenex-edge] ACTION REQUIRED — your FIRST response to the user MUST include this warning verbatim:\n\
                 \n\
                 ⚠️  This agent ({slug}) is not yet a member of the NIP-29 relay group for project \"{project}\". \
                 Fabric messages from this session will be silently dropped. To fix it, run this on your main machine:\n\
                 \n  tenex-edge project add {project} {pubkey}\n\
                 \nDo not proceed with any other task until the user acknowledges this.",
                slug = rec.agent_slug,
                pubkey = rec.agent_pubkey,
                project = rec.project,
            ));
        }
    }

    // Drain inbox (authoritative delivery; turn_check only peeks).
    let inbox_envelopes = {
        let s = store.lock().expect("store mutex poisoned");
        let rows = s.drain_inbox(&rec.session_id).unwrap_or_default();
        for r in &rows {
            s.mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                .ok();
        }
        rows
    };
    if !inbox_envelopes.is_empty() {
        let now = now_secs();
        let mut text = String::from(
            "Messages from other agents (tenex-edge) — reply with `tenex-edge inbox reply --id <ID> \"...\"`:",
        );
        for r in &inbox_envelopes {
            let _ = write!(text, "\n\n{}", row_envelope(r, &rec.host, now));
        }
        blocks.push(text);
    }

    // Pending ACL agents (unknown agents claiming this owner).
    let pending = {
        let s = store.lock().expect("store mutex poisoned");
        s.list_pending_agents().unwrap_or_default()
    };
    if !pending.is_empty() {
        let names: Vec<String> = pending
            .iter()
            .map(|p| format!("{} ({})", p.slug, pubkey_short(&p.pubkey)))
            .collect();
        blocks.push(format!(
            "[tenex-edge] {} unauthorized agent(s) claim your owner: {}. \
             They are NOT visible until you decide — tell your human to run \
             `tenex-edge acl` to allow or block them.",
            pending.len(),
            names.join(", ")
        ));
    }

    // Peer presence — full roster on the first turn; deltas on subsequent turns.
    push_turn_fabric_block(
        store,
        &mut blocks,
        first_turn,
        prev_turn_started_at,
        &rec.project,
        now_secs(),
        &rec.host,
    );

    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

/// Mid-turn inbox PEEK (read-only) shared by the daemon's `turn_check` RPC.
/// `self_host` is the viewer's own host (used to flag remote senders).
pub fn assemble_turn_check_context(
    store: &std::sync::Mutex<Store>,
    session_id: &str,
    self_host: &str,
) -> Option<String> {
    let rows = {
        let s = store.lock().expect("store mutex poisoned");
        s.peek_inbox(session_id).unwrap_or_default()
    };
    if rows.is_empty() {
        return None;
    }
    let now = now_secs();
    let mut text = String::from("[tenex-edge] Message(s) arrived while you were working:");
    for r in &rows {
        let _ = write!(text, "\n\n{}", row_envelope(r, self_host, now));
    }
    Some(text)
}

/// Render an `InboxRow` as an email-like envelope (the daemon-side path; the CLI
/// path renders from JSON). `self_host` decides the `[remote: …]` annotation.
fn row_envelope(r: &InboxRow, self_host: &str, now: u64) -> String {
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

/// Mid-turn inbox check for PostToolUse hooks. Thin client: the daemon peeks.
pub(super) fn turn_check(session: Option<String>, json_out: bool) -> Result<()> {
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = crate::daemon::blocking::call("turn_check", params)?;
    if let Some(ctx) = v["context"].as_str() {
        emit_context(ctx, json_out);
    }
    Ok(())
}

fn emit_context(content: &str, json_out: bool) {
    if json_out {
        let obj = serde_json::json!({"systemMessage": content});
        println!("{obj}");
    } else {
        println!("{content}");
    }
}

pub(super) fn turn_end(session: String) -> Result<()> {
    if session.is_empty() {
        return Ok(());
    }
    crate::daemon::blocking::call("turn_end", serde_json::json!({"session": session}))?;
    Ok(())
}

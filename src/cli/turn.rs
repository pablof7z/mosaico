use super::messaging::{format_mention_line, mention_reply_handle};
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
             `tenex-edge send-message --recipient <agent@project|session-id> --message \"...\"`. \
             If the user asks you to message/contact/tell another agent, run `tenex-edge send-message`; \
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
                "[tenex-edge] WARNING: this agent ({slug}, pubkey {pubkey}) \
                 is not a member of the NIP-29 group for project \"{project}\". \
                 Messages published by this session may be rejected by the relay. \
                 Tell the user to run the following command from a machine that \
                 has relay admin access (e.g. where this project was first set up):\n\
                 \n  tenex-edge project add {project} {pubkey}",
                slug = rec.agent_slug,
                pubkey = rec.agent_pubkey,
                project = rec.project,
            ));
        }
    }

    // Drain inbox (authoritative delivery; turn_check only peeks).
    let inbox_lines = {
        let s = store.lock().expect("store mutex poisoned");
        let rows = s.drain_inbox(&rec.session_id).unwrap_or_default();
        // Render each line (incl. its reply-to handle) while we hold the lock —
        // the handle resolution needs the store.
        rows.iter()
            .map(|r| {
                s.mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                    .ok();
                let handle = mention_reply_handle(&s, r);
                format_mention_line(&r.from_slug, &r.project, &handle, &r.body)
            })
            .collect::<Vec<_>>()
    };
    if !inbox_lines.is_empty() {
        let mut text = String::from("Messages from other agents (tenex-edge):");
        for line in &inbox_lines {
            let _ = write!(text, "\n{line}");
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
pub fn assemble_turn_check_context(
    store: &std::sync::Mutex<Store>,
    session_id: &str,
) -> Option<String> {
    let lines = {
        let s = store.lock().expect("store mutex poisoned");
        let rows = s.peek_inbox(session_id).unwrap_or_default();
        rows.iter()
            .map(|r| {
                let handle = mention_reply_handle(&s, r);
                format_mention_line(&r.from_slug, &r.project, &handle, &r.body)
            })
            .collect::<Vec<_>>()
    };
    if lines.is_empty() {
        return None;
    }
    let mut text = String::from("[tenex-edge] Message(s) arrived while you were working:");
    for line in &lines {
        let _ = write!(text, "\n{line}");
    }
    Some(text)
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

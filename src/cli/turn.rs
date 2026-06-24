use super::who::{build_status_delta, push_turn_fabric_block};
use super::*;

/// How a context block is emitted to the harness on stdout. Selected per
/// (host, hook-type): plain text is injected directly by Claude Code's
/// UserPromptSubmit and opencode; Codex wraps every hook in `{systemMessage}`;
/// Claude Code's PostToolUse only reads context from a `hookSpecificOutput`
/// envelope (plain stdout there is ignored by the harness).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmitFormat {
    PlainText,
    JsonSystemMessage,
    ClaudePostToolUse,
}

// ── turn-start / turn-check / turn-end ───────────────────────────────────────

pub(super) async fn turn_start(
    session: String,
    transcript: Option<String>,
    emit: EmitFormat,
) -> Result<Option<String>> {
    if session.is_empty() {
        return Ok(None);
    }
    let params = serde_json::json!({
        "session": session,
        "transcript": transcript,
    });
    let v = daemon_call_async("turn_start", params).await?;
    if let Some(ctx) = v["context"].as_str() {
        emit_context(ctx, emit);
        return Ok(Some(ctx.to_string()));
    }
    Ok(None)
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
    // Routing scope: channel when set (a `channels switch` moved the session
    // to a subgroup), else the per-session room (`rec.project`). All fabric
    // presence/deltas key on this so a switched session's turn context reflects
    // the new room, not the one it minted at spawn.
    let scope = rec.route_scope().to_string();
    let mut blocks: Vec<String> = Vec::new();

    if first_turn {
        // The agent's identity, orientation, and messaging conventions are now
        // carried by the channel-hierarchy block (push_turn_fabric_block →
        // render_channel_context) appended below — no separate static preamble.

        // Warn only when this daemon is not the local owner for the group. If it
        // owns the group, session-start/room-minting is responsible for signing
        // the member-add itself; a cache miss here is a transient local state,
        // not a user action item.
        let should_warn_not_member = {
            let s = store.lock().expect("store mutex poisoned");
            let not_member = !s.is_group_member(&scope, &rec.agent_pubkey).unwrap_or(true);
            let locally_owned = s.is_group_owned(&scope).unwrap_or(false);
            not_member && !locally_owned
        };
        if should_warn_not_member {
            blocks.push(format!(
                "[tenex-edge] WARNING: this agent ({slug}) \
                 is not a member of the NIP-29 group for project \"{project}\". \
                 Messages published by this session may be rejected by the relay. \
                 Tell the user to run the following command from a machine that \
                 has relay admin access (e.g. where this project was first set up):\n\
                 \n  tenex-edge project add {project} {slug}",
                slug = rec.agent_slug,
                project = scope,
            ));
        }
    }

    let chat_rows = {
        let s = store.lock().expect("store mutex poisoned");
        s.drain_chat(&rec.session_id).unwrap_or_default()
    };
    let (mentions, ambient) = crate::injection::split_direct_mentions(chat_rows, &rec.session_id);
    let now = now_secs();
    if let Some(block) =
        crate::injection::render_direct_mention_prompt(&mentions, &rec.session_id, now)
    {
        blocks.push(block);
    }
    if let Some(block) = crate::injection::render_channel_chat_block(
        "tenex-edge channel messages - reply with `tenex-edge chat write --message \"...\"`:",
        &ambient,
        &rec.session_id,
        now,
    ) {
        blocks.push(block);
    }

    // Peer presence — full roster on the first turn; deltas on subsequent turns.
    push_turn_fabric_block(
        store,
        &mut blocks,
        first_turn,
        prev_turn_started_at,
        &scope,
        now_secs(),
        &rec.host,
        &rec.session_id,
        &rec.agent_slug,
        &rec.agent_pubkey,
    );

    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

/// Mid-turn context for the PostToolUse `turn_check` hook. Three independent
/// blocks, each shown only when it has content:
///   1. Direct mentions — explicit p-tagged user messages, notified once even
///      when the normal awareness delta window is closed.
///   2. Project chat — ambient chat that arrived since the last check.
///      Delta-gated and debounced: shown once per arrival, not on every tool call.
///   3. Sibling-session delta — project-scoped title/status changes since the
///      last check, excluding this session.
/// Ambient chat and sibling deltas are present only when `delta_since` is
/// `Some` (the daemon's rate-limit floor passed) and there is something new
/// past the cursor.
/// `now` is the shared timestamp.
pub fn assemble_turn_check_context(
    store: &std::sync::Mutex<Store>,
    rec: &crate::state::SessionRecord,
    self_host: &str,
    delta_since: Option<u64>,
    now: u64,
) -> Option<String> {
    let mut blocks: Vec<String> = Vec::new();
    // Routing scope: channel when set (a `channels switch` moved the session
    // to a subgroup), else the per-session room. The status delta + chat label
    // key on this so mid-turn context reflects the room the session is actually
    // publishing into after a switch.
    let scope = rec.route_scope().to_string();
    let channel = channel_label(&scope);

    let direct_mentions = {
        let s = store.lock().expect("store mutex poisoned");
        s.peek_unnotified_chat_mentions(&rec.session_id)
            .unwrap_or_default()
    };
    if let Some(block) =
        crate::injection::render_direct_mention_prompt(&direct_mentions, &rec.session_id, now)
    {
        let ids: Vec<String> = direct_mentions
            .iter()
            .map(|row| row.chat_event_id.clone())
            .collect();
        if !ids.is_empty() {
            let s = store.lock().expect("store mutex poisoned");
            s.mark_chat_rows_notified(&rec.session_id, &ids, now).ok();
        }
        blocks.push(block);
    }

    // Ambient chat and sibling-delta remain gated by the daemon's rate-limit
    // floor and cursored off the same `since` so nothing re-emits per tool call.
    if let Some(since) = delta_since {
        let chat_rows: Vec<_> = {
            let s = store.lock().expect("store mutex poisoned");
            s.peek_chat(&rec.session_id).unwrap_or_default()
        }
        .into_iter()
        .filter(|r| r.mentioned_session != rec.session_id)
        // Only chat newer than the cursor is "new since the last check"; older
        // rows were already surfaced this turn (peek leaves them undelivered).
        .filter(|r| r.created_at > since)
        .collect();
        if let Some(block) = crate::injection::render_channel_chat_block(
            &format!("[tenex-edge] Messages on {channel} since your last check:"),
            &chat_rows,
            &rec.session_id,
            now,
        ) {
            blocks.push(block);
        }

        let s = store.lock().expect("store mutex poisoned");
        let delta = build_status_delta(&s, since, &scope, now, self_host, Some(&rec.session_id));
        if !delta.is_empty() {
            blocks.push(format!(
                "tenex-edge fabric — changes on {channel} since your last check:\n{}",
                delta.join("\n")
            ));
        }
    }

    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

fn channel_label(project: &str) -> String {
    let p = project.trim();
    if p.is_empty() {
        "#unknown".to_string()
    } else if p.starts_with('#') {
        p.to_string()
    } else {
        format!("#{p}")
    }
}

/// Mid-turn check for PostToolUse hooks. Thin client: the daemon peeks the
/// inbox and computes the rate-limited sibling-session delta.
pub(super) fn turn_check(session: Option<String>, emit: EmitFormat) -> Result<Option<String>> {
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": agent_env_slug(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = crate::daemon::blocking::call("turn_check", params)?;
    if let Some(ctx) = v["context"].as_str() {
        emit_context(ctx, emit);
        return Ok(Some(ctx.to_string()));
    }
    Ok(None)
}

fn emit_context(content: &str, emit: EmitFormat) {
    match emit {
        EmitFormat::PlainText => println!("{content}"),
        EmitFormat::JsonSystemMessage => {
            let obj = serde_json::json!({ "systemMessage": content });
            println!("{obj}");
        }
        EmitFormat::ClaudePostToolUse => {
            // Claude Code only reads PostToolUse context from this envelope;
            // plain stdout there is ignored by the harness.
            let obj = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": content,
                }
            });
            println!("{obj}");
        }
    }
}

pub(super) fn turn_end(session: String, reply: Option<String>) -> Result<()> {
    if session.is_empty() {
        return Ok(());
    }
    crate::daemon::blocking::call(
        "turn_end",
        serde_json::json!({"session": session, "reply": reply}),
    )?;
    Ok(())
}

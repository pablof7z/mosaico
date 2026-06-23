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
    let mut blocks: Vec<String> = Vec::new();

    if first_turn {
        let codename = crate::util::session_codename(&rec.session_id);
        blocks.push(format!(
            "[tenex-edge] You are {slug} [session {codename}] on the tenex-edge fabric. \
             You can run `tenex-edge whoami` (your own identity), `tenex-edge who`, \
             and `tenex-edge chat write`. \
             To write to project chat: \
             `tenex-edge chat write --message \"...\"`. \
             To mention a specific agent session, write `@<codename>` inline \
             in the `chat write` body. \
             If the user asks you to message/contact/tell another agent, run `tenex-edge chat write`; \
             do not say you cannot send messages from here.",
            slug = rec.agent_slug,
            codename = codename,
        ));

        // Warn only when this daemon is not the local owner for the group. If it
        // owns the group, session-start/room-minting is responsible for signing
        // the member-add itself; a cache miss here is a transient local state,
        // not a user action item.
        let should_warn_not_member = {
            let s = store.lock().expect("store mutex poisoned");
            let not_member = !s
                .is_group_member(&rec.project, &rec.agent_pubkey)
                .unwrap_or(true);
            let locally_owned = s.is_group_owned(&rec.project).unwrap_or(false);
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
                project = rec.project,
            ));
        }
    }

    let chat_rows = {
        let s = store.lock().expect("store mutex poisoned");
        s.drain_chat(&rec.session_id).unwrap_or_default()
    };
    if !chat_rows.is_empty() {
        blocks.push(render_chat_block(
            "tenex-edge project chat - write with `tenex-edge chat write < message.txt`; mention a session by writing `@<codename>` inline in the body:",
            &chat_rows,
            &rec.session_id,
            now_secs(),
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
        &rec.session_id,
    );

    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

/// Mid-turn context for the PostToolUse `turn_check` hook. Two independent
/// blocks, each shown only when it has content:
///   1. Project chat — ambient chat that arrived since the last check.
///      Delta-gated and debounced: shown once per arrival, not on every tool call.
///   2. Sibling-session delta — project-scoped title/status changes since the
///      last check, excluding this session.
/// Both blocks are present only when `delta_since` is `Some` (the daemon's
/// rate-limit floor passed) and there is something new past the cursor.
/// `now` is the shared timestamp.
pub fn assemble_turn_check_context(
    store: &std::sync::Mutex<Store>,
    rec: &crate::state::SessionRecord,
    _self_host: &str,
    delta_since: Option<u64>,
    now: u64,
) -> Option<String> {
    let mut blocks: Vec<String> = Vec::new();

    // Chat and sibling-delta — both gated by the daemon's rate-limit
    // floor and cursored off the same `since` so nothing re-emits per tool call.
    if let Some(since) = delta_since {
        let chat_rows: Vec<_> = {
            let s = store.lock().expect("store mutex poisoned");
            s.peek_chat(&rec.session_id).unwrap_or_default()
        }
        .into_iter()
        // Only chat newer than the cursor is "new since the last check"; older
        // rows were already surfaced this turn (peek leaves them undelivered).
        .filter(|r| r.created_at > since)
        .collect();
        if !chat_rows.is_empty() {
            blocks.push(render_chat_block(
                "[tenex-edge] Project chat while you were working:",
                &chat_rows,
                &rec.session_id,
                now,
            ));
        }

        let s = store.lock().expect("store mutex poisoned");
        let delta = build_status_delta(&s, since, &rec.project, now, Some(&rec.session_id));
        if !delta.is_empty() {
            blocks.push(format!(
                "tenex-edge fabric — changes since your last check:\n{}",
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

/// Mid-turn check for PostToolUse hooks. Thin client: the daemon peeks the
/// inbox and computes the rate-limited sibling-session delta.
pub(super) fn turn_check(session: Option<String>, emit: EmitFormat) -> Result<Option<String>> {
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": agent_env_slug(),
        "channel": crate::cli::channel_env(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = crate::daemon::blocking::call("turn_check", params)?;
    if let Some(ctx) = v["context"].as_str() {
        emit_context(ctx, emit);
        return Ok(Some(ctx.to_string()));
    }
    Ok(None)
}

pub(crate) fn render_chat_block(
    header: &str,
    rows: &[crate::state::ChatInboxRow],
    self_session: &str,
    now: u64,
) -> String {
    let mut text = String::from(header);
    for row in rows {
        // Canonical sender: `codename (agent@host)` when the session is known,
        // else `agent@host`, else the short pubkey for an unknown sender.
        // ChatInboxRow carries no host, so this degrades to `codename (slug)`.
        let from = if row.from_slug.is_empty() {
            pubkey_short(&row.from_pubkey)
        } else if row.from_session.is_empty() {
            row.from_slug.clone()
        } else {
            crate::idref::session_label(&row.from_session, &row.from_slug, "")
        };
        let mention = if row.mentioned_session == self_session {
            " mentioned you"
        } else {
            ""
        };
        let _ = write!(
            text,
            "\n\n{} ({})\n{}{}:\n{}",
            format_local_datetime(row.created_at),
            relative_time(row.created_at, now),
            from,
            mention,
            row.body
        );
        if !row.chat_event_id.is_empty() {
            let _ = write!(text, "\n(message id: {})", pubkey_short(&row.chat_event_id));
        }
    }
    text
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

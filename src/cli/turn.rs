use super::who::{
    render_awareness_snapshot, render_awareness_update_since_check,
    render_awareness_update_since_turn,
};
use super::*;
use crate::state::{InboxRow, RelayEvent, Session};

/// Cap on ambient channel-chat rows pulled from the relay-event log per turn.
const AMBIENT_CHAT_LIMIT: u32 = 50;

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

/// Resolve `nostr:npub1…` mentions in inbox bodies to `@<name>` from the warm
/// profile cache (the daemon warms it from `kind:0` in `rpc_turn_start` before
/// assembly). Sender slugs are no longer stored on the row — readers resolve
/// them from `from_pubkey` at render time via [`crate::profile`].
fn rewrite_inbox_bodies(s: &Store, rows: &mut [InboxRow]) {
    for row in rows.iter_mut() {
        row.body = crate::profile::rewrite_body_mentions(s, &row.body);
    }
}

/// Drain the pending inbound routing ledger for this session and mark each row
/// delivered (idempotency lives in the inbox row's state, not a separate
/// processed table). Bodies get mention-rewritten before they reach the
/// injector.
fn take_inbox(s: &Store, session_id: &str, now: u64) -> Vec<InboxRow> {
    let mut rows = s.drain_pending_for_session(session_id).unwrap_or_default();
    for row in &rows {
        s.mark_delivered(&row.event_id, session_id, now).ok();
    }
    rewrite_inbox_bodies(s, &mut rows);
    rows
}

/// Ambient channel chat from the relay-event log since `since`, oldest-first,
/// excluding events authored by this agent. Replaces the old `peek_chat`
/// inbox-derived ambient stream with the verbatim `relay_events` log.
fn ambient_chat(s: &Store, scope: &str, since: u64, self_pubkey: &str) -> Vec<RelayEvent> {
    s.chat_for_channel(scope, since, AMBIENT_CHAT_LIMIT)
        .unwrap_or_default()
        .into_iter()
        .filter(|ev| ev.pubkey != self_pubkey)
        .collect()
}

/// The full turn-start context assembly, shared by the daemon's `turn_start` RPC
/// (the only caller now). Mutating reads (drain inbox → mark delivered, advance
/// `seen_cursor`) happen here under the shared store; the relay self-fetch is
/// done by the caller beforehand. Single source of truth → injected text cannot
/// drift.
///
/// `backend_pubkey` is this daemon's signing pubkey, used to decide whether we
/// manage (admin) the channel. `prev_turn_started_at` is the `turn_started_at`
/// value BEFORE this turn's mark; the caller passes it so first-turn detection
/// matches the old behavior.
pub fn assemble_turn_start_context(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    backend_pubkey: &str,
    prev_turn_started_at: u64,
) -> Option<String> {
    let first_turn = prev_turn_started_at == 0;
    // Routing scope is the session's `channel_h` — a project channel, or the
    // session/task channel a `channels switch` moved it into. All fabric
    // presence/deltas key on this so a switched session's turn context reflects
    // the channel it actually publishes into.
    let scope = rec.channel_h.clone();
    let now = now_secs();
    let mut blocks: Vec<String> = Vec::new();

    if first_turn {
        // Warn only when this daemon does not manage the channel. If it is an
        // admin, channel/room-minting is responsible for signing the member-add
        // itself; a cache miss here is transient local state, not a user action.
        let should_warn_not_member = {
            let s = store.lock().expect("store mutex poisoned");
            let not_member = !s.is_channel_member(&scope, &rec.agent_pubkey).unwrap_or(true);
            let locally_managed = s.is_channel_admin(&scope, backend_pubkey).unwrap_or(false);
            not_member && !locally_managed
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

    // Direct deliveries (mentions/DMs routed to this session) come from the
    // inbox ledger; ambient channel chat comes from the relay-event log gated by
    // the session's awareness high-water mark (`seen_cursor`).
    let (mentions, ambient) = {
        let s = store.lock().expect("store mutex poisoned");
        let mentions = take_inbox(&s, &rec.session_id, now);
        let ambient = ambient_chat(&s, &scope, rec.seen_cursor, &rec.agent_pubkey);
        (mentions, ambient)
    };
    {
        let s = store.lock().expect("store mutex poisoned");
        if let Some(block) = crate::injection::render_direct_mention_prompt(&s, &mentions, now) {
            blocks.push(block);
        }
        if let Some(block) = crate::injection::render_channel_chat_block(
            &s,
            "tenex-edge channel messages - reply with `tenex-edge chat write --message \"...\"`:",
            &ambient,
            now,
        ) {
            blocks.push(block);
        }
    }

    let awareness = {
        let s = store.lock().expect("store mutex poisoned");
        if first_turn {
            render_awareness_snapshot(&s, &scope, now, &rec.agent_slug, &rec.agent_pubkey)
        } else {
            render_awareness_update_since_turn(
                &s,
                prev_turn_started_at,
                &scope,
                now,
                Some(&rec.agent_pubkey),
            )
        }
    };
    if let Some(block) = awareness {
        blocks.push(block);
    }

    // Advance the awareness high-water mark so the next hook renders only the
    // delta past what we just surfaced.
    {
        let s = store.lock().expect("store mutex poisoned");
        s.set_seen_cursor(&rec.session_id, now).ok();
    }

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
///
///   Ambient chat and sibling deltas are present only when `delta_since` is
///   `Some` (the daemon's rate-limit floor passed) and there is something new
///   past the cursor.
///   `now` is the shared timestamp.
pub fn assemble_turn_check_context(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    _self_host: &str,
    delta_since: Option<u64>,
    now: u64,
) -> Option<String> {
    let mut blocks: Vec<String> = Vec::new();
    // Routing scope is the session's `channel_h`. The status delta + chat label
    // key on this so mid-turn context reflects the channel the session is
    // actually publishing into after a switch.
    let scope = rec.channel_h.clone();
    // The channel's human NAME (never the raw opaque id) for agent-facing labels.
    let channel = {
        let s = store.lock().expect("store mutex poisoned");
        crate::injection::channel_display(&s, &scope)
    };

    // Mentions that arrived mid-turn land as fresh pending inbox rows. Draining
    // them (and marking delivered) is the new "notify once" — there is no
    // separate notified flag; the inbox state IS the idempotency record.
    let direct_mentions = {
        let s = store.lock().expect("store mutex poisoned");
        take_inbox(&s, &rec.session_id, now)
    };
    {
        let s = store.lock().expect("store mutex poisoned");
        if let Some(block) = crate::injection::render_direct_mention_prompt(&s, &direct_mentions, now)
        {
            blocks.push(block);
        }
    }

    // Ambient chat and sibling-delta remain gated by the daemon's rate-limit
    // floor and cursored off the same `since` so nothing re-emits per tool call.
    if let Some(since) = delta_since {
        let s = store.lock().expect("store mutex poisoned");
        let chat_rows = ambient_chat(&s, &scope, since, &rec.agent_pubkey);
        if let Some(block) = crate::injection::render_channel_chat_block(
            &s,
            &format!("[tenex-edge] Messages on {channel} since your last check:"),
            &chat_rows,
            now,
        ) {
            blocks.push(block);
        }

        if let Some(block) =
            render_awareness_update_since_check(&s, since, &scope, now, Some(&rec.agent_pubkey))
        {
            blocks.push(block);
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

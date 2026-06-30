use super::who::{
    render_awareness_snapshot, render_awareness_update_since_check,
    render_awareness_update_since_turn,
};
use super::*;
use crate::state::{InboxRow, RelayEvent, Session};

/// Cap on ambient channel-chat rows pulled from the relay-event log per turn.
const AMBIENT_CHAT_LIMIT: u32 = 50;

/// Walk `channel`'s NIP-29 `parent` links up to the top-level project root (the
/// first channel whose parent is empty/unknown). Bounded against malformed
/// cycles. Mirrors the daemon-side `project_root`, duplicated here because that
/// helper is `pub(in crate::daemon::server)` and this module lives under `cli`.
fn project_root_h(s: &Store, channel: &str) -> String {
    let mut cur = channel.to_string();
    for _ in 0..16 {
        match s.channel_parent(&cur).ok().flatten() {
            Some(p) if !p.is_empty() => cur = p,
            _ => break,
        }
    }
    cur
}

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

/// `degraded_notice` is a visible marker the caller injects when the turn is
/// already known to be degraded (e.g. the session could not be re-registered
/// with the daemon). It is prepended to the daemon's assembled context — and
/// emitted on its own when the daemon returns nothing — so a degraded turn shows
/// a "fabric temporarily unavailable" marker rather than silent emptiness.
pub(super) async fn turn_start(
    session: String,
    transcript: Option<String>,
    emit: EmitFormat,
    degraded_notice: Option<String>,
) -> Result<Option<String>> {
    if session.is_empty() {
        return Ok(None);
    }
    let params = serde_json::json!({
        "session": session,
        "transcript": transcript,
    });
    let v = super::daemon_call_hook_async("turn_start", params).await?;
    let daemon_ctx = v["context"].as_str().map(str::to_string);
    let combined = match (degraded_notice, daemon_ctx) {
        (Some(notice), Some(ctx)) => Some(format!("{notice}\n\n{ctx}")),
        (Some(notice), None) => Some(notice),
        (None, Some(ctx)) => Some(ctx),
        (None, None) => None,
    };
    if let Some(ctx) = combined {
        emit_context(&ctx, emit);
        return Ok(Some(ctx));
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
/// injector. Returns `Err` on a store failure so callers surface a visible
/// "inbox read failed" marker instead of silently rendering an empty inbox —
/// a dropped claim must never look like "no mentions".
fn take_inbox(s: &Store, session_id: &str, now: u64) -> Result<Vec<InboxRow>> {
    // Atomic claim (pending → delivered in one statement). Whoever drains the
    // row first — this hook or the tmux paste path — wins; the other gets
    // nothing. The atomicity IS the dedup: no separate notified flag or gate.
    let mut rows = s.claim_pending_for_session(session_id, now)?;
    rewrite_inbox_bodies(s, &mut rows);
    Ok(rows)
}

/// Ambient channel chat from the relay-event log since `since`, oldest-first,
/// excluding events authored by this agent. Replaces the old `peek_chat`
/// inbox-derived ambient stream with the verbatim `relay_events` log. Returns
/// `Err` on a store failure so a read error is never rendered as a quiet
/// channel.
fn ambient_chat(
    s: &Store,
    scope: &str,
    since: u64,
    self_pubkey: &str,
) -> Result<Vec<RelayEvent>> {
    Ok(s.chat_for_channel(scope, since, AMBIENT_CHAT_LIMIT)?
        .into_iter()
        .filter(|ev| ev.pubkey != self_pubkey)
        .collect())
}

fn joined_channels(s: &Store, rec: &Session) -> Vec<(String, u64)> {
    let mut channels = s
        .list_session_joined_channels(&rec.session_id)
        .unwrap_or_else(|_| vec![(rec.channel_h.clone(), rec.created_at)]);
    if !rec.channel_h.is_empty() && !channels.iter().any(|(h, _)| h == &rec.channel_h) {
        channels.push((rec.channel_h.clone(), rec.created_at));
    }
    channels.sort_by(|(a_h, a_t), (b_h, b_t)| {
        let a_active = if a_h == &rec.channel_h { 0 } else { 1 };
        let b_active = if b_h == &rec.channel_h { 0 } else { 1 };
        a_active.cmp(&b_active).then(a_t.cmp(b_t)).then(a_h.cmp(b_h))
    });
    channels
}

/// Ambient chat grouped per joined channel. The `bool` is `true` when any
/// per-channel read failed: a store error is logged loudly and the channel is
/// dropped from the result, so the caller MUST surface a read-failure marker
/// rather than let a failed read masquerade as a quiet channel.
fn ambient_by_joined_channel(
    s: &Store,
    channels: &[(String, u64)],
    since: u64,
    self_pubkey: &str,
) -> (Vec<(String, Vec<RelayEvent>)>, bool) {
    let mut out = Vec::new();
    let mut read_failed = false;
    for (scope, joined_at) in channels {
        match ambient_chat(s, scope, since.max(*joined_at), self_pubkey) {
            Ok(rows) if !rows.is_empty() => out.push((scope.clone(), rows)),
            Ok(_) => {}
            Err(e) => {
                tracing::error!(
                    channel = %scope,
                    error = ?e,
                    "turn: ambient chat read failed; channel may falsely appear quiet"
                );
                read_failed = true;
            }
        }
    }
    (out, read_failed)
}

fn render_mentions_by_channel(
    s: &Store,
    fallback_scope: &str,
    mentions: &[InboxRow],
    now: u64,
) -> Vec<String> {
    let mut grouped: std::collections::BTreeMap<String, Vec<InboxRow>> =
        std::collections::BTreeMap::new();
    for row in mentions {
        let scope = if row.channel_h.is_empty() {
            fallback_scope
        } else {
            &row.channel_h
        };
        grouped.entry(scope.to_string()).or_default().push(row.clone());
    }
    grouped
        .into_iter()
        .filter_map(|(scope, rows)| {
            let name = crate::injection::channel_display(s, &scope);
            crate::injection::render_hook_mention(s, &name, &rows, now)
        })
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
    self_host: &str,
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
    let joined = {
        let s = store.lock().expect("store mutex poisoned");
        joined_channels(&s, rec)
    };

    if first_turn {
        // Warn only when this daemon does not manage the channel. If it is an
        // admin, channel/room-minting is responsible for signing the member-add
        // itself; a cache miss here is transient local state, not a user action.
        // Compute membership AND the names needed for the warning in one lock.
        let warn = {
            let s = store.lock().expect("store mutex poisoned");
            // A lookup error is NOT membership: treat an Err as "unknown" and
            // fail loud rather than assuming the agent is a member (which would
            // silently suppress the warning when the DB read actually failed).
            let member = match s.is_channel_member(&scope, &rec.agent_pubkey) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(
                        channel = %scope,
                        error = ?e,
                        "turn_start: channel membership lookup failed; cannot confirm membership"
                    );
                    false
                }
            };
            // Likewise, an admin-lookup error must not be read as "we manage it"
            // — that would suppress a legitimate not-a-member warning.
            let locally_managed = match s.is_channel_admin(&scope, backend_pubkey) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(
                        channel = %scope,
                        error = ?e,
                        "turn_start: channel admin lookup failed; cannot confirm management"
                    );
                    false
                }
            };
            (!member && !locally_managed).then(|| {
                let root = project_root_h(&s, &scope);
                let channel_name = crate::injection::channel_display(&s, &scope);
                let project_name = crate::injection::channel_display(&s, &root);
                (root, channel_name, project_name)
            })
        };
        if let Some((root, channel_name, project_name)) = warn {
            // Name the scope precisely: a channel distinct from its project root
            // gets both. When the scope IS the project root, the channel and
            // project coincide and only the project is named.
            let where_label = if root == scope {
                format!("project \"{project_name}\"")
            } else {
                format!("channel \"{channel_name}\" (in project \"{project_name}\")")
            };
            blocks.push(format!(
                "<tenex-edge>\nWARNING: this agent ({slug}) is not a member of the \
                 NIP-29 group for {where_label}. Messages published by this session \
                 may be rejected by the relay. Ask an operator with relay admin \
                 access to add this agent to the channel.\n</tenex-edge>",
                slug = rec.agent_slug,
            ));
        }
    }

    // Direct deliveries (p-tagged mentions) come from the inbox ledger. Ambient
    // channel chat comes from the relay-event log:
    //   - First turn: only messages since this session started (pre-join history
    //     is announced as a compact count, not dumped inline).
    //   - Subsequent turns: messages since the last seen_cursor high-water mark.
    let ambient_since = if first_turn {
        rec.created_at
    } else {
        rec.seen_cursor
    };
    let mut read_failed = false;
    let (mentions, ambient, pre_history_notice) = {
        let s = store.lock().expect("store mutex poisoned");
        // A failed inbox claim must NOT render as an empty inbox: log loudly and
        // flag the turn so a visible marker is injected below.
        let mentions = match take_inbox(&s, &rec.session_id, now) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(
                    session = %rec.session_id,
                    error = ?e,
                    "turn_start: inbox claim failed; direct mentions may be dropped"
                );
                read_failed = true;
                Vec::new()
            }
        };
        let (ambient, ambient_failed) =
            ambient_by_joined_channel(&s, &joined, ambient_since, &rec.agent_pubkey);
        read_failed |= ambient_failed;
        let notice = if first_turn {
            match s.count_channel_events_before(&scope, rec.created_at) {
                Ok(n) if n > 0 => {
                    let name = crate::injection::channel_display(&s, &scope);
                    Some(format!(
                        "<tenex-edge>\n{n} message(s) in #{name} before you joined this session. \
                         Run `tenex-edge chat read` to see them.\n</tenex-edge>"
                    ))
                }
                Ok(_) => None,
                Err(e) => {
                    tracing::error!(
                        channel = %scope,
                        error = ?e,
                        "turn_start: pre-join history count failed"
                    );
                    read_failed = true;
                    None
                }
            }
        } else {
            None
        };
        (mentions, ambient, notice)
    };
    if read_failed {
        blocks.push(
            "<tenex-edge>\n⚠ Fabric read failed while assembling this turn — your inbox \
             and/or channel activity below may be incomplete. Do NOT assume the channel \
             is quiet or that you have no mentions.\n</tenex-edge>"
                .to_string(),
        );
    }
    if let Some(notice) = pre_history_notice {
        blocks.push(notice);
    }
    {
        let s = store.lock().expect("store mutex poisoned");
        for block in render_mentions_by_channel(&s, &scope, &mentions, now) {
            blocks.push(block);
        }
        for (channel_h, rows) in ambient {
            let name = crate::injection::channel_display(&s, &channel_h);
            let ambient_header = if first_turn {
                format!("Activity on #{name} since you joined:")
            } else {
                format!("Activity on #{name} since you last looked:")
            };
            if let Some(block) = crate::injection::render_ambient(&s, &ambient_header, &rows, now)
            {
                blocks.push(block);
            }
        }
    }

    let awareness = {
        let s = store.lock().expect("store mutex poisoned");
        if first_turn {
            render_awareness_snapshot(
                &s,
                &scope,
                now,
                &rec.agent_slug,
                &rec.agent_pubkey,
                self_host,
            )
        } else {
            render_awareness_update_since_turn(
                &s,
                prev_turn_started_at,
                &scope,
                now,
                Some(&rec.agent_pubkey),
                self_host,
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
        if let Err(e) = s.set_seen_cursor(&rec.session_id, now) {
            tracing::error!(
                session = %rec.session_id,
                error = ?e,
                "turn_start: failed to advance seen_cursor; next turn may re-inject already-seen activity"
            );
        }
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
    self_host: &str,
    delta_since: Option<u64>,
    now: u64,
) -> Option<String> {
    let mut blocks: Vec<String> = Vec::new();
    // Routing scope is the session's `channel_h`. The status delta + chat label
    // key on this so mid-turn context reflects the channel the session is
    // actually publishing into after a switch.
    let scope = rec.channel_h.clone();
    let joined = {
        let s = store.lock().expect("store mutex poisoned");
        joined_channels(&s, rec)
    };
    // The channel's human NAME (never the raw opaque id) for agent-facing labels.
    let channel = {
        let s = store.lock().expect("store mutex poisoned");
        crate::injection::channel_display(&s, &scope)
    };

    let mut read_failed = false;
    // Mentions that arrived mid-turn land as fresh pending inbox rows. Draining
    // them (and marking delivered) is the new "notify once" — there is no
    // separate notified flag; the inbox state IS the idempotency record. A
    // failed claim must not silently look like "no mentions".
    let direct_mentions = {
        let s = store.lock().expect("store mutex poisoned");
        match take_inbox(&s, &rec.session_id, now) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(
                    session = %rec.session_id,
                    error = ?e,
                    "turn_check: inbox claim failed; direct mentions may be dropped"
                );
                read_failed = true;
                Vec::new()
            }
        }
    };
    {
        let s = store.lock().expect("store mutex poisoned");
        for block in render_mentions_by_channel(&s, &scope, &direct_mentions, now) {
            blocks.push(block);
        }
    }

    // Ambient chat and sibling-delta remain gated by the daemon's rate-limit
    // floor and cursored off the same `since` so nothing re-emits per tool call.
    if let Some(since) = delta_since {
        let s = store.lock().expect("store mutex poisoned");
        let (ambient, ambient_failed) =
            ambient_by_joined_channel(&s, &joined, since, &rec.agent_pubkey);
        read_failed |= ambient_failed;
        for (channel_h, rows) in ambient {
            let name = crate::injection::channel_display(&s, &channel_h);
            if let Some(block) = crate::injection::render_ambient(
                &s,
                &format!("Activity on #{name} since your last check:"),
                &rows,
                now,
            ) {
                blocks.push(block);
            }
        }

        if let Some(block) = render_awareness_update_since_check(
            &s,
            since,
            &scope,
            now,
            Some(&rec.agent_pubkey),
            self_host,
        ) {
            blocks.push(block);
        }
    }

    if read_failed {
        blocks.insert(
            0,
            "<tenex-edge>\n⚠ Fabric read failed mid-turn — mentions and/or channel \
             activity below may be incomplete.\n</tenex-edge>"
                .to_string(),
        );
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
    if crate::daemon::is_inhibited() {
        return Ok(None);
    }
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
    if session.is_empty() || crate::daemon::is_inhibited() {
        return Ok(());
    }
    crate::daemon::blocking::call(
        "turn_end",
        serde_json::json!({"session": session, "reply": reply}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::state::{RegisterSession, RelayEvent, Store};
    use std::sync::Mutex;

    // Two distinct (fake) pubkeys used throughout — long enough for SQLite but
    // not real Nostr pubkeys (unit tests do not sign or verify).
    const SELF_PK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_PK: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn register(store: &Store, pk: &str, channel: &str, now: u64) -> String {
        store
            .register_session(&RegisterSession {
                harness: "test".into(),
                external_id_kind: "test".into(),
                external_id: format!("{pk}-{now}"),
                agent_pubkey: pk.to_string(),
                agent_slug: "test-agent".into(),
                channel_h: channel.to_string(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now,
            })
            .unwrap()
    }

    fn insert_chat(store: &Store, channel: &str, pubkey: &str, created_at: u64, body: &str) {
        store
            .insert_event(&RelayEvent {
                id: format!("ev-{pubkey}-{created_at}"),
                kind: 9,
                pubkey: pubkey.to_string(),
                created_at,
                channel_h: channel.to_string(),
                d_tag: String::new(),
                content: body.to_string(),
                tags_json: "[]".to_string(),
            })
            .unwrap();
    }

    /// Pre-join history (messages before session.created_at) is announced as a
    /// compact count, never dumped inline.
    #[test]
    fn first_turn_pre_join_history_compact_notice() {
        let m = Mutex::new(Store::open_memory().unwrap());
        let ch = "ch-notice";
        {
            let s = m.lock().unwrap();
            insert_chat(&s, ch, OTHER_PK, 10, "ancient msg 1");
            insert_chat(&s, ch, OTHER_PK, 20, "ancient msg 2");
            insert_chat(&s, ch, OTHER_PK, 30, "ancient msg 3");
        }
        let rec = {
            let s = m.lock().unwrap();
            let id = register(&s, SELF_PK, ch, 100); // session starts at t=100
            s.get_session(&id).unwrap().unwrap()
        };
        let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
        assert!(
            ctx.contains("3 message(s)") && ctx.contains("before you joined"),
            "pre-join history should be announced as a compact count; got:\n{ctx}"
        );
        assert!(
            !ctx.contains("ancient msg 1"),
            "pre-join message content must NOT be dumped inline; got:\n{ctx}"
        );
    }

    /// Messages that arrive between session start and the first turn DO appear
    /// as ambient context (post-join window).
    #[test]
    fn first_turn_post_join_chat_shown_as_ambient() {
        let m = Mutex::new(Store::open_memory().unwrap());
        let ch = "ch-postjoin";
        let rec = {
            let s = m.lock().unwrap();
            let id = register(&s, SELF_PK, ch, 100); // session at t=100
            s.get_session(&id).unwrap().unwrap()
        };
        {
            let s = m.lock().unwrap();
            insert_chat(&s, ch, OTHER_PK, 110, "post-join-message"); // after t=100
        }
        let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
        assert!(
            ctx.contains("post-join-message"),
            "post-join chat should appear in ambient; got:\n{ctx}"
        );
        assert!(
            !ctx.contains("before you joined"),
            "no pre-join notice when channel was empty at join time; got:\n{ctx}"
        );
    }

    /// Channel is completely empty when the session starts and stays empty —
    /// first turn returns no context.
    #[test]
    fn first_turn_empty_channel_no_context() {
        let m = Mutex::new(Store::open_memory().unwrap());
        let ch = "ch-empty";
        let rec = {
            let s = m.lock().unwrap();
            let id = register(&s, SELF_PK, ch, 100);
            s.get_session(&id).unwrap().unwrap()
        };
        // No events at all — should return None (no context blocks).
        let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0);
        assert!(
            ctx.is_none()
                || ctx
                    .as_deref()
                    .map(|s| !s.contains("message") || s.contains("not a member"))
                    .unwrap_or(true),
            "empty channel should produce no message context; got:\n{ctx:?}"
        );
    }

    /// Self-authored messages that predate the session also count toward the
    /// pre-join notice (total channel history, regardless of author).
    #[test]
    fn first_turn_self_authored_pre_join_events_count_for_notice() {
        let m = Mutex::new(Store::open_memory().unwrap());
        let ch = "ch-self-pre";
        {
            let s = m.lock().unwrap();
            insert_chat(&s, ch, SELF_PK, 5, "self-earlier-message");
        }
        let rec = {
            let s = m.lock().unwrap();
            let id = register(&s, SELF_PK, ch, 100);
            s.get_session(&id).unwrap().unwrap()
        };
        let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
        assert!(
            ctx.contains("1 message(s)") && ctx.contains("before you joined"),
            "self-authored pre-join messages should count toward notice; got:\n{ctx}"
        );
    }

    /// Second turn uses the seen_cursor (not session.created_at) for the
    /// ambient window, so messages shown in the first turn don't re-appear.
    #[test]
    fn second_turn_ambient_gates_on_seen_cursor() {
        let m = Mutex::new(Store::open_memory().unwrap());
        let ch = "ch-cursor";
        let sid = {
            let s = m.lock().unwrap();
            register(&s, SELF_PK, ch, 100)
        };
        // Event before session start — surfaces as pre-join notice on first turn.
        {
            let s = m.lock().unwrap();
            insert_chat(&s, ch, OTHER_PK, 50, "pre-join-event");
        }
        // First turn: consumes pre-join notice; seen_cursor → now_secs().
        {
            let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
            let _ = super::assemble_turn_start_context(&m, &rec, "", "", 0);
        }
        // Manually peg the cursor at t=150 so the second turn only sees t>150.
        m.lock().unwrap().set_seen_cursor(&sid, 150).unwrap();
        // Event after the cursor — should appear in the second turn.
        {
            let s = m.lock().unwrap();
            insert_chat(&s, ch, OTHER_PK, 160, "second-turn-event");
        }
        let rec2 = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
        assert_eq!(rec2.seen_cursor, 150, "cursor must be 150 for this test");
        let ctx2 = super::assemble_turn_start_context(
            &m, &rec2, "", "", 1, /* non-zero = not first turn */
        )
        .unwrap_or_default();
        assert!(
            ctx2.contains("second-turn-event"),
            "second turn must show messages since cursor; got:\n{ctx2}"
        );
        // The awareness/activity section independently queries all recent chat,
        // so "pre-join-event" may appear there. Check only the ambient-chat block
        // (the portion before the "[tenex-edge] Fabric updates" awareness header).
        let ambient_portion = ctx2
            .split("[tenex-edge] Fabric updates")
            .next()
            .unwrap_or(&ctx2);
        assert!(
            !ambient_portion.contains("pre-join-event"),
            "pre-cursor message must not appear in the ambient-chat block; got:\n{ambient_portion}"
        );
        assert!(
            !ctx2.contains("before you joined"),
            "pre-join notice must not appear on second turn; got:\n{ctx2}"
        );
    }

    /// An inbox mention (p-tagged, enqueued via enqueue_inbox) appears in the
    /// turn context as a direct-mention block.
    #[test]
    fn inbox_mention_surfaces_in_turn_context() {
        let m = Mutex::new(Store::open_memory().unwrap());
        let ch = "ch-mention";
        let sid = {
            let s = m.lock().unwrap();
            register(&s, SELF_PK, ch, 100)
        };
        {
            let s = m.lock().unwrap();
            s.enqueue_inbox("ev-mention-1", &sid, OTHER_PK, ch, "hey do the thing", 110)
                .unwrap();
        }
        let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
        let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
        assert!(
            ctx.contains("hey do the thing"),
            "inbox mention must appear in turn context; got:\n{ctx}"
        );
    }

    /// Ambient channel chat (not in inbox) is shown alongside an inbox mention
    /// but is labelled as "since you joined", not as a direct mention.
    #[test]
    fn ambient_and_mention_both_in_first_turn_context() {
        let m = Mutex::new(Store::open_memory().unwrap());
        let ch = "ch-dual";
        let sid = {
            let s = m.lock().unwrap();
            register(&s, SELF_PK, ch, 100)
        };
        // Ambient (non-mention) message arriving after session start.
        {
            let s = m.lock().unwrap();
            insert_chat(&s, ch, OTHER_PK, 110, "ambient-background-chat");
        }
        // Direct mention in inbox.
        {
            let s = m.lock().unwrap();
            s.enqueue_inbox("ev-dm-1", &sid, OTHER_PK, ch, "start working on X", 115)
                .unwrap();
        }
        let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
        let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
        assert!(
            ctx.contains("start working on X"),
            "direct mention must appear; got:\n{ctx}"
        );
        assert!(
            ctx.contains("ambient-background-chat"),
            "post-join ambient chat must also appear; got:\n{ctx}"
        );
    }
}

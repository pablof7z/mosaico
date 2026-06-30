//! Turn-context assembly shared by the daemon's `turn_start` / `turn_check`
//! RPCs. This is the single source of truth for the text injected into an
//! agent each turn (membership warnings, inbox mentions, ambient chat, fabric
//! awareness) — kept apart from the thin hook clients in [`super`] so neither
//! file grows past the LOC ceiling.

use super::super::who::{
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

/// Returns `(joined_channels, read_failed)`. On a store error the active channel
/// is still returned as a fallback so the turn is never blank, but `read_failed`
/// is `true` so the caller ORs it into the turn's read-failure flag and surfaces
/// the "⚠ Fabric read failed" marker — a dropped passive channel must not be
/// mistaken for a quiet one.
fn joined_channels(s: &Store, rec: &Session) -> (Vec<(String, u64)>, bool) {
    let (mut channels, read_failed) = match s.list_session_joined_channels(&rec.session_id) {
        Ok(c) => (c, false),
        Err(e) => {
            tracing::error!(
                session = %rec.session_id,
                error = ?e,
                "turn: joined-channel read failed; passive channels may be dropped from this turn"
            );
            (vec![(rec.channel_h.clone(), rec.created_at)], true)
        }
    };
    if !rec.channel_h.is_empty() && !channels.iter().any(|(h, _)| h == &rec.channel_h) {
        channels.push((rec.channel_h.clone(), rec.created_at));
    }
    channels.sort_by(|(a_h, a_t), (b_h, b_t)| {
        let a_active = if a_h == &rec.channel_h { 0 } else { 1 };
        let b_active = if b_h == &rec.channel_h { 0 } else { 1 };
        a_active.cmp(&b_active).then(a_t.cmp(b_t)).then(a_h.cmp(b_h))
    });
    (channels, read_failed)
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
    let (joined, joined_read_failed) = {
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
    // Seed with the joined-channel read result: a failure there silently dropped
    // passive channels, so the marker must fire even if every other read succeeds.
    let mut read_failed = joined_read_failed;
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
            // A count failure must not silently render as "no prior history": log
            // loudly and flag the turn so the read-failure marker fires instead of
            // quietly hiding pre-join messages.
            let n = match s.count_channel_events_before(&scope, rec.created_at) {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!(
                        channel = %scope,
                        error = ?e,
                        "turn_start: pre-join history count failed; prior messages may be hidden"
                    );
                    read_failed = true;
                    0
                }
            };
            if n > 0 {
                let name = crate::injection::channel_display(&s, &scope);
                Some(format!(
                    "<tenex-edge>\n{n} message(s) in #{name} before you joined this session. \
                     Run `tenex-edge chat read` to see them.\n</tenex-edge>"
                ))
            } else {
                None
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
                "turn_start: advancing seen_cursor failed; next turn may re-surface already-shown awareness"
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
    let (joined, joined_read_failed) = {
        let s = store.lock().expect("store mutex poisoned");
        joined_channels(&s, rec)
    };

    let mut read_failed = joined_read_failed;
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

#[cfg(test)]
mod tests;

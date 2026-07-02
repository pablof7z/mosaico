//! Turn-context assembly shared by the daemon's `turn_start` / `turn_check`
//! RPCs. This is the single source of truth for the text injected into an
//! agent each turn (membership warnings, inbox mentions, ambient chat, fabric
//! awareness) — kept apart from the thin hook clients in [`super`] so neither
//! file grows past the LOC ceiling.

use super::super::who::{inbox_seed, render_fabric_context, FabricContextInput};
use super::*;
use crate::state::{InboxRow, RelayEvent, Session};

/// Cap on ambient channel-chat rows pulled from the relay-event log per turn.
const AMBIENT_CHAT_LIMIT: u32 = 50;

fn context_instance(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
) -> crate::identity::AgentInstance {
    store
        .lock()
        .expect("store mutex poisoned")
        .instance_identity_for_session(&rec.session_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            crate::identity::AgentInstance::base(rec.agent_slug.clone(), rec.agent_pubkey.clone())
        })
}

/// Resolve `channel` to the top-level project root through the store-owned
/// channel ancestry helper.
fn project_root_h(s: &Store, channel: &str) -> String {
    s.channel_project_root(channel)
        .ok()
        .flatten()
        .unwrap_or_else(|| channel.to_string())
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
fn ambient_chat(s: &Store, scope: &str, since: u64, self_pubkey: &str) -> Result<Vec<RelayEvent>> {
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
        a_active
            .cmp(&b_active)
            .then(a_t.cmp(b_t))
            .then(a_h.cmp(b_h))
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

/// The full turn-start context assembly, shared by the daemon's `turn_start` RPC
/// (the only caller now). Mutating reads (drain inbox → mark delivered, advance
/// `seen_cursor`) happen here under the shared store; the relay self-fetch is
/// done by the caller beforehand. Single source of truth → injected text cannot
/// drift.
///
/// `backend_pubkey` is this daemon's signing pubkey, used to decide whether we
/// manage (admin) the channel. `_prev_turn_started_at` is retained for the daemon
/// call contract, but first-turn detection is based on `seen_cursor`: `turn_end`
/// clears `turn_started_at`, while `seen_cursor` is the durable injection cursor.
pub fn assemble_turn_start_context(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    backend_pubkey: &str,
    self_host: &str,
    _prev_turn_started_at: u64,
) -> Option<String> {
    let first_turn = rec.seen_cursor == 0;
    // Routing scope is the session's `channel_h` — a project channel, or the
    // session/task channel a `channels switch` moved it into. All fabric
    // presence/deltas key on this so a switched session's turn context reflects
    // the channel it actually publishes into.
    let scope = rec.channel_h.clone();
    let self_instance = context_instance(store, rec);
    let self_slug = self_instance.display_slug();
    let self_pubkey = self_instance.pubkey.clone();
    let now = now_secs();
    let mut warnings: Vec<String> = Vec::new();
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
            let member = match s.is_channel_member(&scope, &self_pubkey) {
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
            warnings.push(format!(
                "WARNING: this agent ({slug}) is not a member of the NIP-29 group \
                 for {where_label}. Messages published by this session may be \
                 rejected by the relay. Ask an operator with relay admin access \
                 to add this agent to the channel.",
                slug = self_slug,
            ));
        }
    }

    // Direct deliveries (p-tagged mentions) come from the inbox ledger. Fabric
    // awareness renders channel chat from the relay-event log:
    //   - First turn: only messages since this session started (pre-join history
    //     is announced as a compact count, not dumped inline).
    //   - Subsequent turns: messages since the last seen_cursor high-water mark.
    // First turn uses session creation time as the ambient floor, but respects
    // any cursor advance that tmux delivery may have written — so messages
    // already injected as the pasted prompt are not re-shown in ambient chat.
    let ambient_since = if first_turn {
        rec.created_at.max(rec.seen_cursor)
    } else {
        rec.seen_cursor
    };
    // Seed with the joined-channel read result: a failure there silently dropped
    // passive channels, so the marker must fire even if every other read succeeds.
    let mut read_failed = joined_read_failed;
    let (mentions, pre_history_notice) = {
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
        let (_ambient, ambient_failed) =
            ambient_by_joined_channel(&s, &joined, ambient_since, &self_pubkey);
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
                    "{n} message(s) in #{name} before you joined this session. \
                     Run `tenex-edge chat read` to see them."
                ))
            } else {
                None
            }
        } else {
            None
        };
        (mentions, notice)
    };
    if read_failed {
        warnings.push(
            "Fabric read failed while assembling this turn; your inbox and/or \
             channel activity below may be incomplete. Do NOT assume the channel \
             is quiet or that you have no mentions."
                .to_string(),
        );
    }
    if let Some(notice) = pre_history_notice {
        warnings.push(notice);
    }

    let forced = mentions.iter().map(inbox_seed).collect::<Vec<_>>();
    let fabric = {
        let s = store.lock().expect("store mutex poisoned");
        render_fabric_context(
            &s,
            FabricContextInput {
                session: Some(rec),
                scope: &scope,
                cursor: rec.seen_cursor,
                now,
                self_slug: &self_slug,
                self_pubkey: &self_pubkey,
                local_host: self_host,
                edge_home: Some(&crate::config::edge_home()),
                forced_messages: &forced,
                warnings: &warnings,
                force: false,
            },
        )
    };

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

    fabric
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
    let mut warnings: Vec<String> = Vec::new();
    // Routing scope is the session's `channel_h`. The status delta + chat label
    // key on this so mid-turn context reflects the channel the session is
    // actually publishing into after a switch.
    let scope = rec.channel_h.clone();
    let self_instance = context_instance(store, rec);
    let self_slug = self_instance.display_slug();
    let self_pubkey = self_instance.pubkey;
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
    // Fabric chat activity and sibling-delta remain gated by the daemon's
    // rate-limit floor and cursored off the same `since` so nothing re-emits
    // per tool call. The joined-channel read stays here only to surface a
    // visible read-failure marker; channel activity text itself is rendered by
    // the unified fabric context below.
    if let Some(since) = delta_since {
        let s = store.lock().expect("store mutex poisoned");
        let (_ambient, ambient_failed) =
            ambient_by_joined_channel(&s, &joined, since, &self_pubkey);
        read_failed |= ambient_failed;
    }

    if read_failed {
        warnings.push(
            "Fabric read failed mid-turn; mentions and/or channel activity below \
             may be incomplete."
                .to_string(),
        );
    }

    if delta_since.is_none() && direct_mentions.is_empty() && warnings.is_empty() {
        return None;
    }
    let forced = direct_mentions.iter().map(inbox_seed).collect::<Vec<_>>();
    let cursor = delta_since.unwrap_or(now);
    let s = store.lock().expect("store mutex poisoned");
    render_fabric_context(
        &s,
        FabricContextInput {
            session: Some(rec),
            scope: &scope,
            cursor,
            now,
            self_slug: &self_slug,
            self_pubkey: &self_pubkey,
            local_host: self_host,
            edge_home: Some(&crate::config::edge_home()),
            forced_messages: &forced,
            warnings: &warnings,
            force: false,
        },
    )
}

#[cfg(test)]
mod tests;

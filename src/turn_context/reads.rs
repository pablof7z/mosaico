use crate::state::{InboxRow, RelayEvent, Session, Store};
use anyhow::Result;

const AMBIENT_CHAT_LIMIT: u32 = 50;

pub(super) fn context_instance(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
) -> crate::identity::SessionIdentity {
    store
        .lock()
        .expect("store mutex poisoned")
        .session_identity_for_session(&rec.session_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            crate::identity::SessionIdentity::fallback(
                &rec.session_id,
                rec.agent_slug.clone(),
                rec.agent_pubkey.clone(),
            )
        })
}

pub(super) fn root_channel_h(s: &Store, channel: &str) -> String {
    s.root_channel_of(channel)
        .ok()
        .flatten()
        .unwrap_or_else(|| channel.to_string())
}

pub(super) fn take_inbox(s: &Store, session_id: &str, now: u64) -> Result<Vec<InboxRow>> {
    // Atomic claim (pending -> delivered in one statement). Whoever drains the
    // row first wins; the inbox state is the idempotency record.
    let mut rows = s.claim_pending_for_session(session_id, now)?;
    rewrite_inbox_bodies(s, &mut rows);
    Ok(rows)
}

pub(super) fn joined_channels(s: &Store, rec: &Session) -> (Vec<(String, u64)>, bool) {
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
    channels.retain(|(channel, _)| !s.is_archived_channel(channel).unwrap_or(false));
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

pub(super) fn ambient_by_joined_channel(
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

fn rewrite_inbox_bodies(s: &Store, rows: &mut [InboxRow]) {
    for row in rows.iter_mut() {
        row.body = crate::profile::rewrite_body_mentions(s, &row.body);
    }
}

fn ambient_chat(s: &Store, scope: &str, since: u64, self_pubkey: &str) -> Result<Vec<RelayEvent>> {
    Ok(s.chat_for_channel(scope, since, AMBIENT_CHAT_LIMIT)?
        .into_iter()
        .filter(|ev| ev.pubkey != self_pubkey)
        .collect())
}

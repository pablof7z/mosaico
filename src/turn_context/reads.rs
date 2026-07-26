use crate::state::{InboxRow, RelayEvent, Session, Store};
use anyhow::Result;

const AMBIENT_CHAT_LIMIT: u32 = 50;

pub(super) fn context_instance(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
) -> crate::identity::SessionIdentity {
    let guard = store.lock().expect("store mutex poisoned");
    guard
        .session_identity(&rec.pubkey)
        .expect("session identity lookup")
        .expect("registered session identity")
}

pub(super) fn take_inbox(s: &Store, target_pubkey: &str, now: u64) -> Result<Vec<InboxRow>> {
    // Atomic claim (pending -> delivered in one statement). Whoever drains the
    // row first wins; the inbox state is the idempotency record.
    let mut rows = s.claim_pending_for_pubkey(target_pubkey, now)?;
    rewrite_inbox_bodies(s, &mut rows);
    Ok(rows)
}

pub(super) fn joined_channels(s: &Store, rec: &Session) -> (Vec<(String, u64)>, bool) {
    let (mut channels, read_failed) = match s.list_session_routes(&rec.pubkey) {
        Ok(c) => (c, false),
        Err(e) => {
            tracing::error!(
                pubkey = %rec.pubkey,
                error = ?e,
                "turn: joined-channel read failed; passive channels may be dropped from this turn"
            );
            (Vec::new(), true)
        }
    };
    channels.retain(|(channel, _)| !s.is_archived_channel(channel).unwrap_or(false));
    channels.sort_by(|(a_h, a_t), (b_h, b_t)| a_t.cmp(b_t).then(a_h.cmp(b_h)));
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
                    "turn: ambient channel read failed; channel may falsely appear quiet"
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
    let mut admitted = Vec::new();
    for event in s.chat_for_channel(scope, since, AMBIENT_CHAT_LIMIT)? {
        if event.pubkey == self_pubkey || event.kind != crate::fabric::nip29::wire::KIND_CHAT as u32
        {
            continue;
        }
        if s.session_membership_admits_event(self_pubkey, scope, &event.id)? {
            admitted.push(event);
        }
    }
    Ok(admitted)
}

use super::notice::{self, Notice};
use super::TaggedRecipient;
use crate::session_state::SessionState;
use crate::state::{Message, Store};
use anyhow::Result;

#[cfg(test)]
#[path = "recipient_notice/tests.rs"]
mod tests;

pub(super) fn suspension_notices(
    store: &Store,
    recipients: &[TaggedRecipient],
) -> Result<Vec<Notice>> {
    let notices = recipients
        .iter()
        .map(|recipient| {
            suspension_notice(
                store,
                &recipient.pubkey,
                &recipient.channel,
                Some(&recipient.label),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(notices.into_iter().flatten().collect())
}

pub(super) fn reply_suspension_notices(store: &Store, original: &Message) -> Result<Vec<Notice>> {
    suspension_notice(store, &original.author_pubkey, &original.channel_h, None)
        .map(|n| n.into_iter().collect())
}

pub(super) fn suspension_notice(
    store: &Store,
    pubkey: &str,
    channel: &str,
    target_label: Option<&str>,
) -> Result<Option<Notice>> {
    let (state, observed_label) = recipient_presence(store, pubkey, channel)?;
    if state != SessionState::Suspended {
        return Ok(None);
    }
    let label = target_label
        .and_then(normalize_label)
        .or_else(|| observed_label.as_deref().and_then(normalize_label));
    let subject = label
        .map(|label| format!("@{label}"))
        .unwrap_or_else(|| "This recipient".to_string());
    Ok(Some(notice::recipient_suspension(label, subject)))
}

fn recipient_presence(
    store: &Store,
    pubkey: &str,
    channel: &str,
) -> Result<(SessionState, Option<String>)> {
    let local = store.get_session(pubkey)?;
    if let Some(session) = local {
        let published = store.get_status(pubkey, channel)?;
        let state = crate::session_presence::local(store, &session, published.as_ref()).state;
        let label = store
            .session_identity(&session.pubkey)?
            .map(|identity| identity.display_slug())
            .or(Some(session.agent_slug));
        return Ok((state, label));
    }

    let status = store.get_status(pubkey, channel)?;
    Ok(match status {
        Some(status) => (
            crate::session_presence::remote(&status).state,
            Some(status.slug),
        ),
        None => (SessionState::Offline, None),
    })
}

fn normalize_label(label: &str) -> Option<&str> {
    let label = label.trim().trim_start_matches('@');
    (!label.is_empty()).then_some(label)
}

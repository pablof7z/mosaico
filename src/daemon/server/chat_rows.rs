use super::*;

pub(super) fn chat_rows_to_json(store: &Store, rows: &[InboxRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter_map(|row| {
            let from_slug = store
                .resolve_slug_for_pubkey(&row.from_pubkey)
                .ok()
                .flatten()
                .unwrap_or_default();
            let channel = crate::channel_ref::full_channel_ref(store, &row.channel_h);
            if channel.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "from_slug": from_slug,
                "channel": channel,
                "host": "",
                "subject": "",
                "created_at": row.created_at,
                "id": crate::idref::event_short_id(&row.event_id),
                "mention_event_id": row.event_id,
                "body": row.body,
                "attachment_dir": row.attachment_dir,
            }))
        })
        .collect()
}

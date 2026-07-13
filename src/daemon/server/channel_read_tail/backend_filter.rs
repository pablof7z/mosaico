use crate::state::{Message, Store};

/// True when `row` is backend↔party traffic (mgmt-command request/response),
/// which must never leak into a shared channel read the way it's already kept
/// out of the hook-injected fabric snapshot (`fabric_context::is_backend_pubkey`,
/// `is_backend_traffic`).
pub(in crate::daemon::server) fn is_backend_row(
    store: &Store,
    backend_pubkey: &str,
    row: &Message,
) -> bool {
    if crate::fabric_context::is_backend_pubkey(store, backend_pubkey, &row.author_pubkey) {
        return true;
    }
    store
        .message_recipients(&row.message_id)
        .unwrap_or_default()
        .iter()
        .any(|r| {
            crate::fabric_context::is_backend_pubkey(store, backend_pubkey, &r.recipient_pubkey)
        })
}

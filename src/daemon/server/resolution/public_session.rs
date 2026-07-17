//! Public local-session selection by authoritative pubkey or leased handle.

use crate::state::{Session, Store};
use anyhow::Result;

/// Resolve only user-facing identity forms. Runtime ids and locator aliases are
/// deliberately excluded; callers that own a typed runtime locator resolve it
/// through the lifecycle path that owns that locator.
pub(in crate::daemon::server) fn resolve(store: &Store, selector: &str) -> Result<Option<Session>> {
    let selector = selector.trim().trim_start_matches('@');
    let pubkey = match crate::idref::normalize_pubkey(selector) {
        Some(pubkey) => Some(pubkey),
        None => store.pubkey_for_handle(selector)?,
    };
    let Some(pubkey) = pubkey else {
        return Ok(None);
    };
    store.get_session(&pubkey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RegisterSession, LOCATOR_NATIVE_RESUME};
    use nostr_sdk::prelude::Keys;

    #[test]
    fn resolves_npub_hex_and_handle_but_not_private_runtime_id() {
        let store = Store::open_memory().unwrap();
        let pubkey = Keys::generate().public_key().to_hex();
        store
            .reserve_hook_session_for_test(&RegisterSession {
                pubkey: pubkey.clone(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                channel_h: "root".into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            })
            .unwrap();
        store
            .put_session_locator("codex", LOCATOR_NATIVE_RESUME, "native-1", &pubkey, 1)
            .unwrap();
        let handle = store.allocate_handle(&pubkey, "codex", 1).unwrap().handle;
        let npub = crate::idref::npub(&pubkey).unwrap();
        let mentioned_handle = format!("@{handle}");

        for selector in [&pubkey, &npub, &handle, &mentioned_handle] {
            assert_eq!(resolve(&store, selector).unwrap().unwrap().pubkey, pubkey);
        }
        assert!(resolve(&store, "native-1").unwrap().is_none());
    }
}

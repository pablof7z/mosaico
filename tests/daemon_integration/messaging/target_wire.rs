use crate::daemon_harness::Home;
use nostr_sdk::prelude::{PublicKey, ToBech32};
use tenex_edge::state::{Session, Store};

pub(super) fn redirected_stdin_body_for_session(
    home: &Home,
    session_id: &str,
    row: &Session,
) -> String {
    let store = Store::open(&home.store_path()).unwrap();
    format!(
        "nostr:{}: hello from redirected stdin",
        target_npub_for_session(&store, session_id, row)
    )
}

pub(super) fn redirected_stdin_rendered_body(codename: &str) -> String {
    format!("@{codename}: hello from redirected stdin")
}

pub(super) fn target_npub_for_session(store: &Store, session_id: &str, row: &Session) -> String {
    let pubkey = store
        .session_identity_for_session(session_id)
        .unwrap()
        .map(|i| i.pubkey)
        .unwrap_or_else(|| row.agent_pubkey.clone());
    PublicKey::from_hex(&pubkey).unwrap().to_bech32().unwrap()
}

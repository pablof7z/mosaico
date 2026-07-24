use crate::daemon_harness::Home;
use mosaico::state::{Session, Store};
use nostr::{PublicKey, ToBech32};

pub(super) fn redirected_stdin_body_for_session(
    home: &Home,
    pubkey: &str,
    row: &Session,
) -> String {
    let store = Store::open(&home.store_path()).unwrap();
    format!(
        "nostr:{}: hello from redirected stdin",
        target_npub_for_session(&store, pubkey, row)
    )
}

pub(super) fn redirected_stdin_rendered_body(codename: &str) -> String {
    format!("@{codename}: hello from redirected stdin")
}

pub(super) fn target_npub_for_session(store: &Store, pubkey: &str, row: &Session) -> String {
    let pubkey = store
        .session_identity(pubkey)
        .unwrap()
        .map(|i| i.pubkey)
        .unwrap_or_else(|| row.pubkey.clone());
    PublicKey::from_hex(&pubkey).unwrap().to_bech32().unwrap()
}

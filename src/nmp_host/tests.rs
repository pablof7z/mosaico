use super::*;
use nostr::key::Keys;

mod boot_recovery;

#[test]
fn configured_local_hosts_are_explicitly_allowed_but_onion_is_not() {
    let local = RelayUrl::parse("ws://127.0.0.1:7777").unwrap();
    let public = RelayUrl::parse("wss://relay.example.com").unwrap();
    let onion = RelayUrl::parse("ws://examplehiddenservice.onion").unwrap();

    assert_eq!(
        local_relay_hosts([&local, &public, &onion]),
        vec!["127.0.0.1"]
    );
}

#[tokio::test]
async fn canonical_nmp_view_stream_has_exactly_one_owner() {
    let host = NmpHost::open(&[], None, None, &Keys::generate()).unwrap();
    let receiver = host.take_view_transitions().unwrap();
    assert!(host.take_view_transitions().is_err());
    drop(receiver);
}

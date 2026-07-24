//! PROBE (ignored by default; run explicitly against the public relay):
//!
//!   MOSAICO_RELAY=wss://relay.tenex.chat \
//!     cargo test --test relay_probe -- --ignored --nocapture
//!
//! Load-bearing question for the per-machine-daemon design: if ONE connection
//! authenticates (NIP-42) as key A, does it still receive events p-tagged to a
//! DIFFERENT key B? If the relay scopes REQ delivery to the connection's authed
//! identity, then collapsing N per-agent connections into one breaks mention
//! delivery for every agent except the one the connection authed as.
//!
//! This talks to a public relay (default wss://relay.tenex.chat, or $MOSAICO_RELAY)
//! and publishes disposable kind:1 probe events. It is not part of default CI or
//! routine local regression tests.

#[path = "common/nmp_client.rs"]
mod nmp_client;

use nmp::AccessContext;
use nmp_client::NmpRelayClient;
use nostr::*;
use std::time::Duration;

fn relay_url() -> String {
    std::env::var("MOSAICO_RELAY").unwrap_or_else(|_| "wss://relay.tenex.chat".to_string())
}

#[tokio::test]
#[ignore]
async fn one_authed_conn_receives_mentions_to_other_pubkeys() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let relay = relay_url();
    eprintln!("[probe] relay = {relay}");

    // The "daemon" connection authenticates as agent A.
    let key_a = Keys::generate();
    let key_b = Keys::generate(); // a second local agent the daemon also hosts
    let pk_b = key_b.public_key();
    eprintln!("[probe] daemon authed as A={}", key_a.public_key().to_hex());
    eprintln!("[probe] subscribing for mentions to B={}", pk_b.to_hex());

    let daemon = NmpRelayClient::connect(key_a.clone(), &relay)
        .await
        .expect("connect daemon NMP client");

    // Subscribe (on the A-authed connection) to kind:1 events p-tagging B.
    let sub = Filter::new().kind(Kind::from(1u16)).pubkey(pk_b);
    let subscription = daemon
        .observe(sub, AccessContext::Nip42(key_a.public_key()))
        .expect("subscribe through NMP");

    // A separate sender (key C) publishes a kind:1 p-tagging B.
    let key_c = Keys::generate();
    let sender = NmpRelayClient::connect(key_c.clone(), &relay)
        .await
        .expect("connect sender NMP client");
    let marker = format!("mosaico-probe-{}", key_c.public_key().to_hex());
    let builder = EventBuilder::new(Kind::from(1u16), &marker).tags([Tag::public_key(pk_b)]);
    sender
        .send_event_builder(builder)
        .await
        .expect("send mention to B");
    eprintln!("[probe] sent mention to B with marker {marker}");

    // Did the A-authed NMP observation receive it?
    let expected = marker.clone();
    let got = tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Ok(frame) = subscription.recv_timeout(remaining) else {
                return false;
            };
            if frame
                .deltas
                .iter()
                .filter_map(|delta| delta.event())
                .any(|event| event.content == expected)
            {
                return true;
            }
        }
    })
    .await
    .expect("join NMP observation");

    daemon.disconnect().await;
    sender.disconnect().await;

    eprintln!(
        "[probe] RESULT: one A-authed connection {} mentions p-tagged to B",
        if got { "RECEIVES" } else { "DOES NOT RECEIVE" }
    );
    assert!(
        got,
        "one-connection design REQUIRES this: an A-authed connection must \
         receive events p-tagged to B. If this fails, the daemon needs a relay \
         connection per hosted agent pubkey (or the relay must not scope \
         delivery by authed identity)."
    );
}

#[tokio::test]
#[ignore]
async fn one_conn_publishes_events_signed_by_multiple_keys() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let relay = relay_url();

    // Daemon connection authed as A.
    let key_a = Keys::generate();
    let key_b = Keys::generate(); // a second hosted agent
    let mut daemon = NmpRelayClient::connect(key_a.clone(), &relay)
        .await
        .expect("connect daemon NMP client");
    daemon
        .register_identity(&key_b)
        .expect("register second hosted identity");

    // Publish an event SIGNED BY B over the A-authed connection via send_event.
    let marker = format!("mosaico-probe-multisign-{}", key_b.public_key().to_hex());
    let signed = EventBuilder::new(Kind::from(1u16), &marker)
        .sign_with_keys(&key_b)
        .expect("sign with B");
    let res = daemon.send_event(&signed).await;
    eprintln!("[probe] publish B-signed over A-conn: {res:?}");

    // Read it back as a fresh reader to confirm it landed under B's pubkey.
    let reader = NmpRelayClient::connect(Keys::generate(), &relay)
        .await
        .expect("connect reader NMP client");
    let f = Filter::new()
        .kind(Kind::from(1u16))
        .author(key_b.public_key())
        .limit(5);
    let evs = reader
        .fetch_events(f, Duration::from_secs(5))
        .await
        .unwrap_or_default();
    let found = evs
        .into_iter()
        .any(|e| e.content == marker && e.pubkey == key_b.public_key());

    daemon.disconnect().await;
    reader.disconnect().await;
    eprintln!(
        "[probe] RESULT: B-signed event over A-conn {} land under B's authorship",
        if found { "DID" } else { "DID NOT" }
    );
    assert!(
        found,
        "multi-agent publish requires send_event(pre-signed) to work over the shared connection"
    );
}

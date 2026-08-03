use super::*;
use axum::{body::Bytes, routing::get, Router};
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// `["attachment", url, label, sha256]`. The digest is the bytes the fixture
/// server actually serves, because the receiver verifies before it writes.
fn attachment_tag(url: &str, label: &str, bytes: &[u8]) -> Tag {
    Tag::parse([
        "attachment",
        url,
        label,
        &nmp_asset::Sha256Hash::of(bytes).to_hex(),
    ])
    .unwrap()
}

fn register_receiver(state: &DaemonState, pubkey: &str) {
    state.with_store(|store| {
        register(store, pubkey, "reviewer", "room", "locator");
        store.upsert_channel("room", "room", "", "", 1).unwrap();
    });
}

#[tokio::test]
async fn remote_attachment_is_persisted_before_direct_inbox_routing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/report",
                get(|| async { Bytes::from_static(b"finished plan") }),
            ),
        )
        .await
        .unwrap()
    });
    let state = DaemonState::new_for_test().await;
    let receiver_pk = Keys::generate().public_key().to_hex();
    register_receiver(&state, &receiver_pk);
    let event = EventBuilder::new(Kind::from(9), "Done. [plan/report.md]")
        .tags([
            Tag::parse(["h", "room"]).unwrap(),
            Tag::parse(["p", receiver_pk.as_str()]).unwrap(),
            attachment_tag(
                &format!("http://{address}/report"),
                "plan/report.md",
                b"finished plan",
            ),
        ])
        .sign_with_keys(&Keys::generate())
        .unwrap();

    inbound_dispatch::handle_for_test(&state, &event).await;

    let (message, inbox) = state.with_store(|store| {
        (
            store.get_message(&event.id.to_hex()).unwrap().unwrap(),
            store.peek_pending_for_pubkey(&receiver_pk).unwrap(),
        )
    });
    assert!(!message.attachment_dir.is_empty());
    assert_eq!(inbox[0].attachment_dir, message.attachment_dir);
    assert_eq!(
        std::fs::read(std::path::Path::new(&message.attachment_dir).join("plan/report.md"))
            .unwrap(),
        b"finished plan"
    );
    server.abort();
}

#[tokio::test]
async fn attachment_failure_still_routes_the_ordinary_message() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, Router::new()).await.unwrap() });
    let state = DaemonState::new_for_test().await;
    let receiver_pk = Keys::generate().public_key().to_hex();
    register_receiver(&state, &receiver_pk);
    let event = EventBuilder::new(Kind::from(9), "Keep this body")
        .tags([
            Tag::parse(["h", "room"]).unwrap(),
            Tag::parse(["p", receiver_pk.as_str()]).unwrap(),
            attachment_tag(
                &format!("http://{address}/missing"),
                "missing.md",
                b"never served",
            ),
        ])
        .sign_with_keys(&Keys::generate())
        .unwrap();

    inbound_dispatch::handle_for_test(&state, &event).await;

    state.with_store(|store| {
        let message = store.get_message(&event.id.to_hex()).unwrap().unwrap();
        assert_eq!(message.body, "Keep this body");
        assert!(message.attachment_dir.is_empty());
        let inbox = store.peek_pending_for_pubkey(&receiver_pk).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].body, "Keep this body");
        assert!(inbox[0].attachment_dir.is_empty());
    });
    server.abort();
}

#[tokio::test]
async fn slow_attachment_does_not_stall_demux_or_duplicate_the_download() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let requests = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let route_entered = entered.clone();
    let route_release = release.clone();
    let route_requests = requests.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/slow",
                get(move || {
                    let entered = route_entered.clone();
                    let release = route_release.clone();
                    let requests = route_requests.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        entered.notify_one();
                        release.notified().await;
                        Bytes::from_static(b"eventual file")
                    }
                }),
            ),
        )
        .await
        .unwrap()
    });
    let state = DaemonState::new_for_test().await;
    let receiver_pk = Keys::generate().public_key().to_hex();
    register_receiver(&state, &receiver_pk);
    let sender = Keys::generate();
    let slow = EventBuilder::new(Kind::from(9), "Slow file")
        .tags([
            Tag::parse(["h", "room"]).unwrap(),
            Tag::parse(["p", receiver_pk.as_str()]).unwrap(),
            attachment_tag(
                &format!("http://{address}/slow"),
                "slow.md",
                b"eventual file",
            ),
        ])
        .sign_with_keys(&sender)
        .unwrap();

    inbound_dispatch::dispatch(&state, &slow);
    inbound_dispatch::dispatch(&state, &slow);
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let fast = EventBuilder::new(Kind::from(9), "Fast ordinary message")
        .tags([
            Tag::parse(["h", "room"]).unwrap(),
            Tag::parse(["p", receiver_pk.as_str()]).unwrap(),
        ])
        .sign_with_keys(&sender)
        .unwrap();
    inbound_dispatch::dispatch(&state, &fast);

    state.with_store(|store| {
        assert!(store
            .get_message(&slow.id.to_hex())
            .unwrap()
            .unwrap()
            .attachment_dir
            .is_empty());
        let inbox = store.peek_pending_for_pubkey(&receiver_pk).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].event_id, fast.id.to_hex());
    });
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let ready = state.with_store(|store| {
                let materialized = store
                    .get_message(&slow.id.to_hex())
                    .unwrap()
                    .is_some_and(|message| !message.attachment_dir.is_empty());
                let routed = store
                    .peek_pending_for_pubkey(&receiver_pk)
                    .unwrap()
                    .iter()
                    .any(|row| row.event_id == slow.id.to_hex());
                materialized && routed
            });
            if ready {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.abort();
}

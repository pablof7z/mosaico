//! The Blossom upload path, end to end against a mock server.

use super::super::*;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::put;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use nostr::{Event, JsonUtil};
use std::sync::Mutex;

#[derive(Clone, Debug, Default)]
struct CapturedRequest {
    authorization: String,
    content_type: String,
    hash: String,
    body: Vec<u8>,
}

async fn test_server(
    public_name: &str,
) -> (
    String,
    Arc<Mutex<CapturedRequest>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}/");
    let public_url = format!("{base}{public_name}");
    let capture = Arc::new(Mutex::new(CapturedRequest::default()));
    let handler_capture = capture.clone();
    let app = Router::new().route(
        "/upload",
        put(move |headers: HeaderMap, body: Bytes| {
            let capture = handler_capture.clone();
            let public_url = public_url.clone();
            async move {
                let bytes = body.to_vec();
                *capture.lock().unwrap() = CapturedRequest {
                    authorization: headers[reqwest::header::AUTHORIZATION]
                        .to_str()
                        .unwrap()
                        .to_string(),
                    content_type: headers[reqwest::header::CONTENT_TYPE]
                        .to_str()
                        .unwrap()
                        .to_string(),
                    hash: headers["x-sha-256"].to_str().unwrap().to_string(),
                    body: bytes.clone(),
                };
                // A complete BUD-02 descriptor. NMP parses strictly and gates
                // the returned sha256 against the bytes it actually sent.
                (
                    StatusCode::CREATED,
                    axum::Json(serde_json::json!({
                        "url": public_url,
                        "sha256": Sha256Hash::of(&bytes).to_hex(),
                        "size": bytes.len(),
                    })),
                )
            }
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base.replacen("http://", "ws://", 1), capture, task)
}

/// A host that admits the loopback Blossom server, exactly as a daemon
/// configured against a local relay does.
fn local_host(relay: &str) -> Arc<NmpHost> {
    Arc::new(NmpHost::open(&[relay.to_string()], None, None, &Keys::generate()).unwrap())
}

// ── mosaico#742: the upload is nmp-blossom's ─────────────────────────────────

#[tokio::test]
async fn uploads_through_nmp_blossom_and_returns_a_verified_hash() {
    let bytes = b"\x89PNG\r\nattachment";
    let (relay, capture, server) = test_server("blob.png").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("diagram.png");
    std::fs::write(&file, bytes).unwrap();
    let keys = Keys::generate();
    let host = local_host(&relay);

    let uploaded = upload_all(
        &[Attachment {
            label: "1/diagram.png".into(),
            path: file,
        }],
        &[relay],
        &host,
        &keys,
    )
    .await
    .unwrap();

    assert_eq!(
        uploaded,
        vec![ChatAttachment {
            url: uploaded[0].url.clone(),
            label: "1/diagram.png".into(),
            sha256: Sha256Hash::of(bytes).to_hex(),
        }],
        "the sender now carries the hash it computed instead of discarding it"
    );
    assert!(uploaded[0].url.ends_with("/blob.png"));

    let request = capture.lock().unwrap().clone();
    assert_eq!(request.body, bytes);
    assert_eq!(request.content_type, "image/png");
    assert_eq!(request.hash, Sha256Hash::of(bytes).to_hex());

    // BUD-11 on the wire, composed and encoded by NMP: `Nostr <base64url,
    // no padding>` — not the standard-alphabet base64 the hand-rolled client
    // emitted.
    let encoded = request.authorization.strip_prefix("Nostr ").unwrap();
    let event_json = String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
    let event = Event::from_json(event_json).unwrap();
    event.verify().unwrap();
    assert_eq!(event.kind.as_u16(), 24242);
    assert_eq!(
        event.pubkey,
        keys.public_key(),
        "the grant is authored by the exact chat identity"
    );
    let value = serde_json::to_value(event).unwrap();
    let tags = value["tags"].as_array().unwrap();
    assert!(tags.contains(&serde_json::json!(["t", "upload"])));
    assert!(tags.contains(&serde_json::json!(["x", request.hash])));
    assert!(tags.iter().any(|tag| tag[0] == "expiration"));
    server.abort();
}

/// The whole reason to adopt the crate rather than keep the local client: the
/// grant is signed through `NmpHost`, never with raw `Keys`. This was the only
/// production `sign_with_keys` in the repo, and it is why a non-local signer
/// could not produce an attachment at all.
#[tokio::test]
async fn the_authorization_is_signed_through_nmp_not_with_raw_keys() {
    let (relay, capture, server) = test_server("blob.bin").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("trace.bin");
    std::fs::write(&file, b"trace").unwrap();
    let keys = Keys::generate();
    let host = local_host(&relay);

    upload_all(
        &[Attachment {
            label: "trace.bin".into(),
            path: file,
        }],
        &[relay],
        &host,
        &keys,
    )
    .await
    .unwrap();

    let encoded = capture.lock().unwrap().authorization.clone();
    let encoded = encoded.strip_prefix("Nostr ").unwrap();
    let event =
        Event::from_json(String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap())
            .unwrap();
    assert!(
        host.has_registered_identity(&keys.public_key()),
        "signing the grant must go through NMP's identity registry"
    );
    assert_eq!(event.pubkey, keys.public_key());
    server.abort();
}

/// A server that answers a different blob's descriptor is refused, and nothing
/// is returned to the caller. The hand-rolled client parsed only `url` and had
/// no way to notice.
#[tokio::test]
async fn a_descriptor_whose_hash_does_not_match_the_bytes_is_refused() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("ws://{address}/");
    let app = Router::new().route(
        "/upload",
        put(|| async {
            (
                StatusCode::CREATED,
                axum::Json(serde_json::json!({
                    "url": "https://cdn.example.com/somebody-elses-blob",
                    "sha256": Sha256Hash::of(b"not what was uploaded").to_hex(),
                    "size": 21,
                })),
            )
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("trace.bin");
    std::fs::write(&file, b"trace").unwrap();
    let host = local_host(&base);

    let error = upload_all(
        &[Attachment {
            label: "trace.bin".into(),
            path: file,
        }],
        &[base],
        &host,
        &Keys::generate(),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("refusing the descriptor") && error.contains("hashing to"),
        "{error}"
    );
    server.abort();
}

#[tokio::test]
async fn surfaces_server_error_with_its_reason() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("ws://{address}/");
    let app = Router::new().route(
        "/upload",
        put(|| async { (StatusCode::FORBIDDEN, "only group members can upload blobs") }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("trace.bin");
    std::fs::write(&file, b"trace").unwrap();
    let host = local_host(&base);

    let error = upload_all(
        &[Attachment {
            label: "trace.bin".into(),
            path: file,
        }],
        &[base],
        &host,
        &Keys::generate(),
    )
    .await
    .unwrap_err()
    .to_string();
    // NMP separates an authorization refusal from an ordinary server refusal.
    // 403 is the former; the reason it reports is the BUD-01 `X-Reason` header
    // rather than an arbitrary body, which this fixture does not send.
    assert!(error.contains("403"), "{error}");
    assert!(error.contains("rejected the authorization"), "{error}");
    server.abort();
}

/// A descriptor missing a mandatory BUD-02 field is refused by NMP's strict
/// parser. The hand-rolled client accepted any JSON carrying a `url`.
#[tokio::test]
async fn rejects_an_incomplete_success_descriptor() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("ws://{address}/");
    let app = Router::new().route(
        "/upload",
        put(|| async {
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({"url": "not enough"})),
            )
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("trace.bin");
    std::fs::write(&file, b"trace").unwrap();
    let host = local_host(&base);

    let error = upload_all(
        &[Attachment {
            label: "trace.bin".into(),
            path: file,
        }],
        &[base],
        &host,
        &Keys::generate(),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("missing the mandatory `sha256`"), "{error}");
    server.abort();
}

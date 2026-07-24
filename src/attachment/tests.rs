use super::*;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::put;
use axum::Router;
use nostr::Event;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
struct CapturedRequest {
    authorization: String,
    content_type: String,
    hash: String,
    body: Vec<u8>,
}

async fn test_server(
    expected_hash: String,
    expected_size: usize,
) -> (
    String,
    Arc<Mutex<CapturedRequest>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}/");
    let public_url = format!("{base}{expected_hash}.png");
    let capture = Arc::new(Mutex::new(CapturedRequest::default()));
    let handler_capture = capture.clone();
    let app = Router::new().route(
        "/upload",
        put(move |headers: HeaderMap, body: Bytes| {
            let capture = handler_capture.clone();
            let public_url = public_url.clone();
            let expected_hash = expected_hash.clone();
            async move {
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
                    body: body.to_vec(),
                };
                (
                    StatusCode::CREATED,
                    axum::Json(serde_json::json!({
                        "url": public_url,
                        "sha256": expected_hash,
                        "size": expected_size,
                        "type": "image/png",
                        "uploaded": 1,
                    })),
                )
            }
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base.replacen("http://", "ws://", 1), capture, task)
}

#[test]
fn parses_label_and_path_at_first_equals() {
    let parsed = parse_spec("diagram=out/a=b.png").unwrap();
    assert_eq!(parsed.label, "diagram");
    assert_eq!(parsed.path, PathBuf::from("out/a=b.png"));
}

#[test]
fn rejects_invalid_specs() {
    for raw in ["diagram", "=file.png", "bad label=file.png", "diagram="] {
        assert!(parse_spec(raw).is_err(), "accepted {raw:?}");
    }
}

#[test]
fn derives_blossom_server_from_primary_relay() {
    assert_eq!(
        blossom_server(&["wss://nip29.f7z.io/".into()])
            .unwrap()
            .as_str(),
        "https://nip29.f7z.io/"
    );
    assert_eq!(
        blossom_server(&["ws://localhost:8080/api/".into()])
            .unwrap()
            .as_str(),
        "http://localhost:8080/"
    );
}

#[tokio::test]
async fn validates_labels_before_network_access() {
    let keys = Keys::generate();
    let duplicate = vec![
        Attachment {
            label: "x".into(),
            path: "a".into(),
        },
        Attachment {
            label: "x".into(),
            path: "b".into(),
        },
    ];
    assert!(upload_and_expand("[x]", &duplicate, &[], &keys)
        .await
        .unwrap_err()
        .to_string()
        .contains("duplicate attachment label"));

    let missing = vec![Attachment {
        label: "x".into(),
        path: "a".into(),
    }];
    assert!(upload_and_expand("no marker", &missing, &[], &keys)
        .await
        .unwrap_err()
        .to_string()
        .contains("does not reference attachment [x]"));
}

#[tokio::test]
async fn uploads_signed_blob_and_replaces_every_label_occurrence() {
    let bytes = b"\x89PNG\r\nattachment";
    let hash = format!("{:x}", Sha256::digest(bytes));
    let (relay, capture, server) = test_server(hash.clone(), bytes.len()).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("diagram.png");
    std::fs::write(&file, bytes).unwrap();
    let keys = Keys::generate();
    let attachments = vec![Attachment {
        label: "diagram".into(),
        path: file,
    }];

    let expanded = upload_and_expand(
        "See [diagram], then compare [diagram].",
        &attachments,
        &[relay],
        &keys,
    )
    .await
    .unwrap();
    assert!(!expanded.contains("[diagram]"));
    assert_eq!(expanded.matches(&hash).count(), 2);

    let request = capture.lock().unwrap().clone();
    assert_eq!(request.body, bytes);
    assert_eq!(request.content_type, "image/png");
    assert_eq!(request.hash, hash);
    let encoded = request.authorization.strip_prefix("Nostr ").unwrap();
    let event_json = String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap();
    let event = Event::from_json(event_json).unwrap();
    event.verify().unwrap();
    assert_eq!(event.kind, Kind::Custom(24242));
    assert_eq!(event.pubkey, keys.public_key());
    let value = serde_json::to_value(event).unwrap();
    let tags = value["tags"].as_array().unwrap();
    assert!(tags.contains(&serde_json::json!(["t", "upload"])));
    assert!(tags.contains(&serde_json::json!(["x", request.hash])));
    assert!(tags.contains(&serde_json::json!(["server", "127.0.0.1"])));
    assert!(tags.iter().any(|tag| tag[0] == "expiration"));
    server.abort();
}

#[tokio::test]
async fn surfaces_server_error_without_expanding_message() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/upload",
        put(|| async { (StatusCode::FORBIDDEN, "only group members can upload blobs") }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("trace.bin");
    std::fs::write(&file, b"trace").unwrap();

    let error = upload_and_expand(
        "[trace]",
        &[Attachment {
            label: "trace".into(),
            path: file,
        }],
        &[format!("ws://{address}/")],
        &Keys::generate(),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("HTTP 403 Forbidden"), "{error}");
    assert!(error.contains("only group members"), "{error}");
    server.abort();
}

#[tokio::test]
async fn rejects_malformed_success_descriptor() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
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

    let error = upload_and_expand(
        "[trace]",
        &[Attachment {
            label: "trace".into(),
            path: file,
        }],
        &[format!("ws://{address}/")],
        &Keys::generate(),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("parsing Blossom response"));
    server.abort();
}

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
                    axum::Json(serde_json::json!({"url": public_url})),
                )
            }
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base.replacen("http://", "ws://", 1), capture, task)
}

#[test]
fn infers_relative_label_from_supplied_path() {
    let parsed = parse_spec("./1/a=b.png").unwrap();
    assert_eq!(parsed.label, "1/a=b.png");
    assert_eq!(parsed.path, PathBuf::from("./1/a=b.png"));
}

#[test]
fn absolute_path_uses_just_the_file_name_as_label() {
    let parsed = parse_spec("/tmp/build/trace.json").unwrap();
    assert_eq!(parsed.label, "trace.json");
}

#[test]
fn rejects_empty_parent_and_unsafe_marker_paths() {
    for raw in ["", "../secret", "out/../secret", "bad[name].png"] {
        assert!(parse_spec(raw).is_err(), "accepted {raw:?}");
    }
}

#[test]
fn prepares_missing_labels_without_replacing_existing_brackets() {
    let attachments = vec![
        Attachment {
            label: "report.md".into(),
            path: "report.md".into(),
        },
        Attachment {
            label: "1/screenshot.png".into(),
            path: "1/screenshot.png".into(),
        },
    ];
    assert_eq!(
        prepare_message("Review [report.md].", &attachments).unwrap(),
        "Review [report.md].\n\n[1/screenshot.png]"
    );
}

#[test]
fn rejects_duplicate_and_overlapping_labels() {
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
    assert!(prepare_message("chat", &duplicate)
        .unwrap_err()
        .to_string()
        .contains("duplicate attachment label"));

    let overlapping = vec![
        Attachment {
            label: "output".into(),
            path: "a".into(),
        },
        Attachment {
            label: "output/log.txt".into(),
            path: "b".into(),
        },
    ];
    assert!(prepare_message("chat", &overlapping)
        .unwrap_err()
        .to_string()
        .contains("overwrite the same path"));
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
async fn uploads_signed_blob_and_returns_label_url_mapping() {
    let bytes = b"\x89PNG\r\nattachment";
    let (relay, capture, server) = test_server("blob.png").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("diagram.png");
    std::fs::write(&file, bytes).unwrap();
    let keys = Keys::generate();
    let uploaded = upload_all(
        &[Attachment {
            label: "1/diagram.png".into(),
            path: file,
        }],
        &[relay],
        &keys,
    )
    .await
    .unwrap();

    assert_eq!(
        uploaded,
        vec![ChatAttachment {
            url: uploaded[0].url.clone(),
            label: "1/diagram.png".into(),
        }]
    );
    assert!(uploaded[0].url.ends_with("/blob.png"));

    let request = capture.lock().unwrap().clone();
    assert_eq!(request.body, bytes);
    assert_eq!(request.content_type, "image/png");
    assert_eq!(request.hash, format!("{:x}", Sha256::digest(bytes)));
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
async fn surfaces_server_error() {
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

    let error = upload_all(
        &[Attachment {
            label: "trace.bin".into(),
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

    let error = upload_all(
        &[Attachment {
            label: "trace.bin".into(),
            path: file,
        }],
        &[format!("ws://{address}/")],
        &Keys::generate(),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("invalid Blossom URL"));
    server.abort();
}

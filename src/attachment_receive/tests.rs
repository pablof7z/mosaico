use super::*;
use axum::body::Bytes;
use axum::routing::get;
use axum::Router;

async fn server(bytes: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/file",
        get(move || async move { Bytes::from_static(bytes) }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}/file"), task)
}

#[tokio::test]
async fn downloads_nested_labels_and_reuses_redelivery_directory() {
    let root = tempfile::tempdir().unwrap();
    let (url, task) = server(b"diagram").await;
    let attachments = vec![ChatAttachment {
        url,
        label: "images/diagram.png".into(),
    }];

    let first = download(root.path(), "abcdef111111", &attachments)
        .await
        .unwrap()
        .unwrap();
    let second = download(root.path(), "abcdef111111", &attachments)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first, root.path().join("abcdef"));
    assert_eq!(second, first);
    assert_eq!(
        std::fs::read(first.join("images/diagram.png")).unwrap(),
        b"diagram"
    );
    task.abort();
}

#[test]
fn extends_short_id_on_collision_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let first = event_directory(root.path(), "abcdef111111").unwrap();
    let second = event_directory(root.path(), "abcdef222222").unwrap();
    assert_eq!(first.file_name().unwrap(), "abcdef");
    assert_eq!(second.file_name().unwrap(), "abcdef2");

    let destination = first.join("report.md");
    write_new(&destination, b"first").unwrap();
    write_new(&destination, b"second").unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"first");
}

#[tokio::test]
async fn reserved_marker_labels_are_rejected_before_receive_storage() {
    let root = tempfile::tempdir().unwrap().path().join("attachments");
    for label in [".event-id", ".EVENT-ID/file.png"] {
        let error = download(
            &root,
            "abcdef111111",
            &[ChatAttachment {
                url: "https://example.invalid/file".into(),
                label: label.into(),
            }],
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("reserved path component"));
    }
    assert!(!root.exists());
}

#[test]
fn reserved_marker_labels_are_rejected_before_local_copy() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::write(&source, b"file").unwrap();
    let root = directory.path().join("attachments");
    let error = copy_local(
        &root,
        "abcdef111111",
        &[crate::attachment::Attachment {
            label: ".event-id/nested".into(),
            path: source,
        }],
    )
    .unwrap_err();
    assert!(error.to_string().contains("reserved path component"));
    assert!(!root.exists());
}

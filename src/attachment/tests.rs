use super::*;

#[test]
fn parses_explicit_label_and_preserves_its_inline_reference() {
    let parsed = parse_spec("review=./build/report.pdf").unwrap();
    assert_eq!(parsed.label, "review");
    assert_eq!(parsed.path, PathBuf::from("./build/report.pdf"));
    assert_eq!(
        prepare_message("Check out [review].", &[parsed]).unwrap(),
        "Check out [review]."
    );
}

#[test]
fn infers_relative_label_from_supplied_path() {
    let parsed = parse_spec("./1/a=b.png").unwrap();
    assert_eq!(parsed.label, "1/a=b.png");
    assert_eq!(parsed.path, PathBuf::from("./1/a=b.png"));
}

#[test]
fn absolute_path_uses_just_the_file_name_as_label() {
    let parsed = parse_spec("/tmp/build/trace=a.json").unwrap();
    assert_eq!(parsed.label, "trace=a.json");
}

#[test]
fn rejects_empty_parent_and_unsafe_marker_paths() {
    for raw in [
        "",
        "../secret",
        "out/../secret",
        "bad[name].png",
        ".event-id",
        ".EVENT-ID/file.png",
    ] {
        assert!(parse_spec(raw).is_err(), "accepted {raw:?}");
    }
}

#[test]
fn rejects_incomplete_or_unsafe_explicit_labels() {
    for raw in [
        "=./report.pdf",
        "review=",
        "../review=./report.pdf",
        "bad[label]=./report.pdf",
        ".event-id=./report.pdf",
    ] {
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

    let case_collision = vec![
        Attachment {
            label: "Images/A.png".into(),
            path: "a".into(),
        },
        Attachment {
            label: "images/a.PNG".into(),
            path: "b".into(),
        },
    ];
    assert!(prepare_message("chat", &case_collision)
        .unwrap_err()
        .to_string()
        .contains("duplicate attachment label"));
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

#[path = "tests/upload.rs"]
mod upload;

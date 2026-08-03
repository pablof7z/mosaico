use super::*;

fn digest() -> String {
    "aa".repeat(32)
}

fn decode_attachment_tags(tags: &[(&str, &str)]) -> ChatMessage {
    let keys = Keys::generate();
    let mut wire_tags = vec![Tag::parse(["h", "mychannel"]).unwrap()];
    wire_tags.extend(
        tags.iter()
            .map(|(url, label)| Tag::parse(["attachment", url, label, &digest()]).unwrap()),
    );
    let signed = EventBuilder::new(kind(KIND_CHAT), "still ordinary chat")
        .tags(wire_tags)
        .sign_with_keys(&keys)
        .unwrap();
    match Nip29WireCodec.decode_event(&signed) {
        Some(DomainEvent::ChatMessage(chat)) => chat,
        other => panic!("malformed attachments dropped the whole chat: {other:?}"),
    }
}

#[test]
fn encodes_repeated_pubkey_mentions() {
    let keys = Keys::generate();
    let first_pk = "dd".repeat(32);
    let second_pk = "ee".repeat(32);
    let event = DomainEvent::ChatMessage(ChatMessage {
        from: agent(&keys, "codex"),
        channel: "mychannel".into(),
        body: "status: tests are green".into(),
        mentioned_pubkeys: vec![first_pk.clone(), second_pk.clone()],
        attachments: vec![crate::domain::ChatAttachment {
            url: "https://blossom.example/diagram.png".into(),
            label: "1/diagram.png".into(),
            sha256: digest(),
        }],
    });
    let codec = Nip29WireCodec;
    let signed = codec
        .encode_event(&event)
        .expect("encode")
        .sign_with_keys(&keys)
        .expect("sign");

    assert_eq!(signed.kind.as_u16(), KIND_CHAT);
    assert!(has_tag(&signed, "h", "mychannel"));
    assert!(has_tag(&signed, "p", &first_pk));
    assert!(has_tag(&signed, "p", &second_pk));
    assert!(signed.tags.iter().any(|tag| {
        tag.as_slice()
            == [
                "attachment",
                "https://blossom.example/diagram.png",
                "1/diagram.png",
                &digest(),
            ]
    }));
    match codec.decode_event(&signed) {
        Some(DomainEvent::ChatMessage(chat)) => {
            assert_eq!(chat.channel, "mychannel");
            assert_eq!(chat.body, "status: tests are green");
            assert_eq!(chat.mentioned_pubkeys, vec![first_pk, second_pk]);
            assert_eq!(chat.attachments.len(), 1);
            assert_eq!(chat.attachments[0].label, "1/diagram.png");
        }
        other => panic!("expected ChatMessage, got {other:?}"),
    }
}

#[test]
fn decode_ignores_parent_and_absolute_attachment_labels() {
    let chat = decode_attachment_tags(&[
        ("https://blossom.example/parent", "../secret.txt"),
        ("https://blossom.example/absolute", "/tmp/secret.txt"),
        ("https://blossom.example/good", "evidence/good.txt"),
    ]);

    assert_eq!(chat.body, "still ordinary chat");
    assert_eq!(
        chat.attachments,
        vec![crate::domain::ChatAttachment {
            url: "https://blossom.example/good".into(),
            label: "evidence/good.txt".into(),
            sha256: digest(),
        }]
    );
}

#[test]
fn decode_keeps_the_first_duplicate_attachment_label() {
    let chat = decode_attachment_tags(&[
        ("https://blossom.example/first", "result.txt"),
        ("https://blossom.example/second", "result.txt"),
    ]);

    assert_eq!(chat.attachments.len(), 1);
    assert_eq!(chat.attachments[0].url, "https://blossom.example/first");
}

#[test]
fn decode_drops_attachment_labels_that_overlap_paths() {
    let parent_first = decode_attachment_tags(&[
        ("https://blossom.example/parent", "output"),
        ("https://blossom.example/child", "output/log.txt"),
    ]);
    let child_first = decode_attachment_tags(&[
        ("https://blossom.example/child", "output/log.txt"),
        ("https://blossom.example/parent", "output"),
    ]);

    assert_eq!(parent_first.attachments.len(), 1);
    assert_eq!(parent_first.attachments[0].label, "output");
    assert_eq!(child_first.attachments.len(), 1);
    assert_eq!(child_first.attachments[0].label, "output/log.txt");
}

#[test]
fn decode_ignores_non_http_attachment_urls() {
    let chat = decode_attachment_tags(&[
        ("file:///tmp/secret", "secret.txt"),
        ("ftp://files.example/result", "result.txt"),
        ("https://blossom.example/good", "good.txt"),
    ]);

    assert_eq!(chat.attachments.len(), 1);
    assert_eq!(chat.attachments[0].label, "good.txt");
}

#[test]
fn decode_ignores_incomplete_and_overlong_attachment_tags() {
    let keys = Keys::generate();
    let signed = EventBuilder::new(kind(KIND_CHAT), "still ordinary chat")
        .tags([
            Tag::parse(["h", "mychannel"]).unwrap(),
            Tag::parse(["attachment", "https://blossom.example/missing-label"]).unwrap(),
            // Three values is now INCOMPLETE, not complete: an attachment with
            // no declared digest cannot be verified on receipt, so it is not an
            // attachment this build will accept.
            Tag::parse([
                "attachment",
                "https://blossom.example/no-digest",
                "no-digest.txt",
            ])
            .unwrap(),
            Tag::parse([
                "attachment",
                "https://blossom.example/extra",
                "extra.txt",
                &digest(),
                "unexpected",
            ])
            .unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();

    match Nip29WireCodec.decode_event(&signed) {
        Some(DomainEvent::ChatMessage(chat)) => {
            assert_eq!(chat.body, "still ordinary chat");
            assert!(chat.attachments.is_empty());
        }
        other => panic!("malformed attachments dropped the whole chat: {other:?}"),
    }
}

#[test]
fn encode_rejects_invalid_domain_attachments() {
    let keys = Keys::generate();
    let event = DomainEvent::ChatMessage(ChatMessage {
        from: agent(&keys, "codex"),
        channel: "mychannel".into(),
        body: "chat survives only after fixing metadata".into(),
        mentioned_pubkeys: Vec::new(),
        attachments: vec![crate::domain::ChatAttachment {
            url: "file:///tmp/secret".into(),
            label: "../secret".into(),
            sha256: digest(),
        }],
    });

    assert!(Nip29WireCodec.encode_event(&event).is_err());
}

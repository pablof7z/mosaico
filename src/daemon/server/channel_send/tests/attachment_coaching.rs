use super::super::*;

#[test]
fn attachment_labels_survive_tag_normalization_and_ack_uses_display_text() {
    let tagged = vec![TaggedRecipient {
        label: "agent1".into(),
        pubkey: "379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe".into(),
        channel: "room".into(),
    }];
    let attachment = crate::attachment::Attachment {
        label: "report.md".into(),
        path: "report.md".into(),
    };
    let prepared = prepare_outbound_message("Agent1: Review the report.", &[attachment]).unwrap();
    let formatted = body::format_tagged_body(&prepared, &tagged).unwrap();

    assert_eq!(
        formatted.message, "Review the report.\n\n[report.md]",
        "prepared bracket labels remain in normalized display text"
    );
    assert!(formatted
        .wire
        .ends_with(": Review the report.\n\n[report.md]"));
    assert_eq!(formatted.stripped_label.as_deref(), Some("agent1"));
    assert!(coaching::ack_like(&formatted.message).is_none());

    let ack = body::format_tagged_body("AGENT1: Got it!", &tagged).unwrap();
    assert_eq!(ack.message, "Got it!");
    assert!(coaching::ack_like(&ack.message).is_some());
}

#[test]
fn accepted_publish_persists_files_without_creating_delivery() {
    let store = crate::state::Store::open_memory().unwrap();
    persist_attachment_directory("published-event", || {
        store.set_message_attachment_dir(
            "published-event",
            std::path::Path::new("/tmp/mosaico-files/published-event"),
        )
    });

    assert!(store.get_message("published-event").unwrap().is_none());
    assert!(store
        .peek_pending_for_pubkey("recipient")
        .unwrap()
        .is_empty());
}

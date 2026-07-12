use super::*;

#[test]
fn routes_every_repeated_p_tag() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let first = Keys::generate();
    let second = Keys::generate();
    let sender_pk = sender.public_key().to_hex();
    let first_pk = first.public_key().to_hex();
    let second_pk = second.public_key().to_hex();
    let first_sid = register(&store, &first_pk, "proj", "first-ext");
    let second_sid = register(&store, &second_pk, "proj", "second-ext");
    let event = build(
        &sender,
        9,
        "both of you",
        vec![
            make_tag(&["h", "proj"]),
            make_tag(&["p", &first_pk]),
            make_tag(&["p", &second_pk]),
        ],
    );
    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(sender_pk, String::new()),
        channel: "proj".into(),
        body: "both of you".into(),
        mentioned_pubkeys: vec![first_pk, second_pk],
    };

    assert!(Nip29Materializer::route_chat(&store, &event, &chat));
    assert_eq!(store.peek_pending_for_session(&first_sid).unwrap().len(), 1);
    assert_eq!(
        store.peek_pending_for_session(&second_sid).unwrap().len(),
        1
    );
}

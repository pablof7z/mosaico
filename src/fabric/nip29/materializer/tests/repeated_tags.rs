use super::*;

#[test]
fn parks_every_distinct_p_tag() {
    let store = Store::open_memory().unwrap();
    let sender = Keys::generate();
    let first = Keys::generate();
    let second = Keys::generate();
    let sender_pk = sender.public_key().to_hex();
    let first_pk = first.public_key().to_hex();
    let second_pk = second.public_key().to_hex();
    register(&store, &first_pk, "proj", "first-ext");
    register(&store, &second_pk, "proj", "second-ext");
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
        mentioned_pubkeys: vec![first_pk.clone(), second_pk.clone()],
    };

    assert!(Nip29Materializer::materialize_event(&store, &event));
    assert!(route_for_test(&store, &event, &chat));
    assert_eq!(store.peek_pending_for_pubkey(&first_pk).unwrap().len(), 1);
    assert_eq!(store.peek_pending_for_pubkey(&second_pk).unwrap().len(), 1);
}

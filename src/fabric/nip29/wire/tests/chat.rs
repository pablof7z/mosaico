use super::*;

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
    match codec.decode_event(&signed) {
        Some(DomainEvent::ChatMessage(chat)) => {
            assert_eq!(chat.channel, "mychannel");
            assert_eq!(chat.body, "status: tests are green");
            assert_eq!(chat.mentioned_pubkeys, vec![first_pk, second_pk]);
        }
        other => panic!("expected ChatMessage, got {other:?}"),
    }
}

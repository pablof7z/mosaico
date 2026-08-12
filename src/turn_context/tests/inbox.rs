use super::*;

/// An inbox mention (p-tagged, enqueued via enqueue_inbox) appears in the
/// turn context as a direct-mention block.
#[test]
fn inbox_mention_surfaces_in_turn_context() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-mention";
    let sid = {
        let s = m.lock().unwrap();
        materialize_channel(&s, ch);
        register(&s, SELF_PK, ch, 100)
    };
    {
        let s = m.lock().unwrap();
        let mention = mention_event(&s, "ev-mention-1", ch, 110, "hey do the thing");
        s.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([mention]));
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx =
        super::super::render_turn_start_text_for_test(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("hey do the thing"),
        "inbox mention must appear in turn context; got:\n{ctx}"
    );
}

/// Ambient channel chat (not in inbox) is shown alongside an inbox mention in
/// the same structured fabric context.
#[test]
fn ambient_and_mention_both_in_first_turn_context() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-dual";
    let now = crate::util::now_secs().saturating_sub(100);
    let sid = {
        let s = m.lock().unwrap();
        materialize_channel(&s, ch);
        register(&s, SELF_PK, ch, now)
    };
    {
        let s = m.lock().unwrap();
        s.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([chat_event(
            ch,
            OTHER_PK,
            now + 10,
            "ambient-background-chat",
        )]));
    }
    {
        let s = m.lock().unwrap();
        let mention = mention_event(&s, "ev-dm-1", ch, now + 15, "start working on X");
        s.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
            chat_event(ch, OTHER_PK, now + 10, "ambient-background-chat"),
            mention,
        ]));
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx =
        super::super::render_turn_start_text_for_test(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("start working on X"),
        "direct mention must appear; got:\n{ctx}"
    );
    assert!(
        ctx.contains("ambient-background-chat"),
        "post-join ambient chat must also appear; got:\n{ctx}"
    );
    assert!(
        ctx.contains("<chatter>")
            && ctx.contains(
                "Follow up on ev-dm-: reply for substantive context or react for an ACK."
            ),
        "ambient chat and mention must render in the fabric context; got:\n{ctx}"
    );
}

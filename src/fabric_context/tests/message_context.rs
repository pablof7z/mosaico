use super::*;

fn reaction(id: &str, target: &str, channel: &str, reactor: &str, at: u64) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind: crate::fabric::nip29::wire::KIND_REACTION as u32,
        pubkey: reactor.into(),
        created_at: at,
        channel_h: channel.into(),
        d_tag: String::new(),
        content: "👍".into(),
        tags_json: format!(r#"[["e","{target}"]]"#),
    }
}

#[test]
fn automatic_context_requires_both_join_fences() {
    let store = seed_store();
    let future = chat(
        "future-before-join",
        "root",
        500,
        "future-dated prejoin body",
        "[]",
    );
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .events([future.clone()]),
    );
    let rec = session(&store);
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().profiles(seed_profiles()).events([
            future,
            chat(
                "backdated-after-join",
                "root",
                5,
                "backdated postjoin body",
                "[]",
            ),
            chat("valid-after-join", "root", 210, "valid postjoin body", "[]"),
        ]),
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 600, true))
        .expect("forced context should render");
    assert!(!text.contains("future-dated prejoin body"), "got: {text}");
    assert!(!text.contains("backdated postjoin body"), "got: {text}");
    assert!(text.contains("valid postjoin body"), "got: {text}");
}

#[test]
fn mention_rows_are_marked_important_and_truncated_with_recovery_id() {
    let store = seed_store();
    let rec = session(&store);
    let body = (0..305)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let tags = format!("[[\"p\",\"{SELF_PK}\"]]");
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().profiles(seed_profiles()).events([
            chat("mention-long", "root", 210, &body, &tags),
            reaction("rx-mention-long", "mention-long", "root", SELF_PK, 211),
        ]),
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("mention should render");
    assert!(text.contains("<channels>"));
    assert!(!text.contains("<workspace"));
    assert!(text.contains("<channel name=\"#root\""));
    assert!(!text.contains("<channel name=\"#root\" id=\""));
    assert!(text.contains("<message from=\"@reviewer\" id=\"mentio\""));
    assert!(text.contains("age=\"1m\""));
    assert!(!text.contains("Follow up on mentio:"));
    assert!(!text.contains("mention=\"true\""));
    assert!(!text.contains("truncated=\"true\""));
    assert!(text.contains("[message truncated; run `mosaico channel read --id mentio`]"));
    assert!(text.contains("<important>"));
    assert!(text.contains("<mention channel=\"#root\""));
    assert!(text.contains("message_id=\"mentio\""));
}

#[test]
fn mention_rows_without_followup_show_compact_affordance() {
    let store = seed_store();
    let rec = session(&store);
    let tags = format!("[[\"p\",\"{SELF_PK}\"]]");
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .events([chat(
                "mention-guide",
                "root",
                210,
                "please review this",
                &tags,
            )]),
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("mention should render");

    assert!(
        text.contains("Follow up on mentio: reply for substantive context or react for an ACK."),
        "got: {text}"
    );
    assert!(
        !text.contains("references/coordination-guide.md"),
        "got: {text}"
    );
}

#[test]
fn injected_mention_row_is_hidden_from_chatter() {
    let store = seed_store();
    let rec = session(&store);
    let tags = format!("[[\"p\",\"{SELF_PK}\"]]");
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .events([chat(
                "mention-inj",
                "root",
                210,
                "please pick this up",
                &tags,
            )]),
    );

    store
        .enqueue_inbox(
            "mention-inj",
            &rec.pubkey,
            OTHER_PK,
            "root",
            "please pick this up",
            210,
        )
        .unwrap();
    store.claim_pending_for_pubkey(&rec.pubkey, 210).unwrap();
    store
        .mark_injected_for_echo(&["mention-inj".to_string()], &rec.pubkey, 210)
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, true))
        .expect("forced context should still render");
    assert!(!text.contains("please pick this up"));
}

#[test]
fn message_rows_show_p_tag_recipients_and_rewrite_nostr_mentions() {
    use nostr::{PublicKey, ToBech32};

    const TARGET_PK: &str = "379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe";
    const REMOTE_PK: &str = "9aa6883eee2f1ce43053a1eec2c1c8b1c712cbb3c77ec346d9f091982a50b461";

    let store = seed_store();
    let rec = session(&store);
    let mut profiles = seed_profiles();
    profiles.push(profile(
        TARGET_PK,
        "target@laptop",
        "target",
        "",
        "laptop",
        false,
    ));
    profiles.push(profile(
        REMOTE_PK,
        "remote@tower",
        "remote",
        "",
        "tower",
        false,
    ));
    let npub = PublicKey::from_hex(TARGET_PK).unwrap().to_bech32().unwrap();
    let tags = format!("[[\"p\",\"{TARGET_PK}\"],[\"p\",\"{REMOTE_PK}\"]]");
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles(profiles).events([
        chat(
            "mention-target",
            "root",
            210,
            &format!("please ask nostr:{npub} for review"),
            &tags,
        ),
    ]));

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("p-tagged ambient message should render");
    assert!(
        text.contains("for=\"@target @remote@tower\""),
        "got: {text}"
    );
    assert!(text.contains("please ask @target@laptop for review"));
    assert!(!text.contains("nostr:npub"), "got: {text}");

    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 200, 300));
    assert_eq!(rendered, text);
}

#[test]
fn ambient_attachment_renders_the_canonical_directory_without_child_nodes() {
    let store = seed_store();
    let rec = session(&store);
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .events([chat(
                "attach-event",
                "root",
                210,
                "Review [plan/report.md]",
                "[]",
            )]),
    );
    store
        .set_message_attachment_dir(
            "attach-event",
            std::path::Path::new("/tmp/mosaico-files/attach"),
        )
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("attachment message should render");
    assert!(text.contains("attachment-dir=\"/tmp/mosaico-files/attach\""));
    assert!(text.contains("Review [plan/report.md]"));
    assert!(!text.contains("<attachment"));
}

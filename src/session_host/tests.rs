fn sample_session() -> crate::state::Session {
    crate::state::Session {
        pubkey: "pk-target".into(),
        runtime_generation: 1,
        agent_slug: "claude".into(),
        channel_h: "proj".into(),
        work_root: "proj".into(),
        readiness_parent: String::new(),
        observed_harness: "claude".into(),
        claimed_harness: String::new(),
        admitted_bundle: String::new(),
        admitted_transport: String::new(),
        endpoint_provenance: "hook".to_string(),
        child_pid: None,
        transcript_path: None,
        alive: true,
        created_at: 1000,
        last_seen: 0,
        working: false,
        turn_started_at: 0,
        seen_cursor: 0,
        title: String::new(),
        explicit_chat_published_at: 0,
    }
}

#[test]
fn pending_message_prompt_contains_the_actual_message_body() {
    let rec = sample_session();
    // Renderer shows the short sender pubkey.
    let row = crate::state::InboxRow {
        event_id: "abcdef123456".into(),
        target_pubkey: rec.pubkey.clone(),
        state: "pending".into(),
        from_pubkey: "pk-sender".into(),
        channel_h: "proj".into(),
        body: "please review the PTY delivery path".into(),
        created_at: 100,
        delivered_at: 0,
    };

    // No whitelist → the sender is treated as another agent. With no cached slug
    // the name falls back to the short sender pubkey ("pk-sende"), and with no
    // channel metadata the source room is still the workspace's general channel.
    let store = crate::state::Store::open_memory().unwrap();
    let prompt = crate::injection::render_terminal_mention(&store, &[row], &[], 120).unwrap();

    assert_eq!(
        prompt,
        "<mosaico>\n\
         \u{20}\u{20}<channel ref=\"proj\">\n\
         \u{20}\u{20}\u{20}\u{20}<message from=\"@pk-sende\" id=\"abcdef\">please review the PTY delivery path</message>\n\
         \u{20}\u{20}</channel>\n\
         \n\
         \u{20}\u{20}Reply via: `mosaico channel reply abcdef --message \"hello world\"`\n\
         \u{20}\u{20}Attachments: add `--attach label=/path/to/file` and reference `[label]` in the message.\n\
         </mosaico>"
    );
}

#[test]
fn whitelisted_human_mention_renders_bare_with_provenance() {
    let rec = sample_session();
    let row = crate::state::InboxRow {
        event_id: "ev-human".into(),
        target_pubkey: rec.pubkey.clone(),
        state: "pending".into(),
        from_pubkey: "human-pk".into(),
        channel_h: "channel-writer-test".into(),
        body: "@developer hey there".into(),
        created_at: 100,
        delivered_at: 0,
    };
    let store = crate::state::Store::open_memory().unwrap();
    store
        .upsert_channel("mosaico", "mosaico", "", "", 1)
        .unwrap();
    store
        .upsert_channel("channel-writer-test", "writer-test", "", "mosaico", 100)
        .unwrap();
    // Sender is whitelisted, but the injected line still carries the source room.
    let prompt =
        crate::injection::render_terminal_mention(&store, &[row], &["human-pk".into()], 120)
            .unwrap();
    assert_eq!(
        prompt,
        "<mosaico>\n\
         \u{20}\u{20}<channel ref=\"mosaico.writer-test\">\n\
         \u{20}\u{20}\u{20}\u{20}<message from=\"@human-pk\" id=\"ev-hum\">@developer hey there</message>\n\
         \u{20}\u{20}</channel>\n\
         \n\
         \u{20}\u{20}Reply via: `mosaico channel reply ev-hum --message \"hello world\"`\n\
         \u{20}\u{20}Attachments: add `--attach label=/path/to/file` and reference `[label]` in the message.\n\
         </mosaico>"
    );
}

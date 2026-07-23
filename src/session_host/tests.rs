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
        runtime_state: crate::state::RuntimeState::Running,
        presentation_state: crate::state::PresentationState::Headed,
        work_state: crate::state::WorkState::Idle,
        recovery_state: crate::state::RecoveryState::Pending,
        lifecycle_epoch: 1,
        attachment_epoch: 1,
        idle_since: 0,
        idle_deadline: 0,
        stopped_at: 0,
        stop_reason: None,
        turn_count: 0,
        created_at: 1000,
        last_seen: 0,
        turn_started_at: 0,
        seen_cursor: 0,
        title: String::new(),
        explicit_chat_published_at: 0,
        state_changed_at: 0,
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

#[test]
fn multiple_whitelisted_humans_render_as_distinct_named_senders() {
    let store = crate::state::Store::open_memory().unwrap();
    store
        .upsert_channel("workspace", "workspace", "", "", 1)
        .unwrap();
    let humans = [
        ("pk-pablo", "Pablo", "PABLO-TOKEN"),
        ("pk-alice", "Alice", "ALICE-TOKEN"),
        ("pk-bob", "Bob", "BOB-TOKEN"),
    ];
    let rows = humans
        .iter()
        .enumerate()
        .map(|(index, (pubkey, name, token))| {
            store
                .upsert_profile(pubkey, name, name, "", false, 100)
                .unwrap();
            crate::state::InboxRow {
                event_id: format!("event-{index}"),
                target_pubkey: "pk-target".into(),
                state: "pending".into(),
                from_pubkey: (*pubkey).into(),
                channel_h: "workspace".into(),
                body: (*token).into(),
                created_at: 100,
                delivered_at: 0,
            }
        })
        .collect::<Vec<_>>();
    let whitelist = humans
        .iter()
        .map(|(pubkey, _, _)| (*pubkey).to_string())
        .collect::<Vec<_>>();

    let prompt = crate::injection::render_terminal_mention(&store, &rows, &whitelist, 120)
        .expect("multi-human prompt");
    for (_, name, token) in humans {
        assert!(
            prompt.contains(&format!("<message from=\"@{name}\"")),
            "missing distinct sender label for {name}: {prompt}"
        );
        assert!(prompt.contains(token), "missing {token}: {prompt}");
    }
}

use crate::state::{Profile, RelayEvent, TestGroup, TestGroupDelivery, TestRelayDelivery};

#[path = "tests/multiple.rs"]
mod multiple;

fn relay_event(
    id: &str,
    kind: u32,
    pubkey: &str,
    channel: &str,
    content: &str,
    created_at: u64,
    tags_json: &str,
) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind,
        pubkey: pubkey.into(),
        created_at,
        channel_h: channel.into(),
        d_tag: String::new(),
        content: content.into(),
        tags_json: tags_json.into(),
    }
}

fn sample_session() -> crate::state::Session {
    crate::state::Session {
        pubkey: "pk-target".into(),
        runtime_generation: 1,
        agent_slug: "claude".into(),
        work_root: "proj".into(),
        readiness_parent: String::new(),
        observed_harness: "claude".into(),
        claimed_harness: String::new(),
        admitted_bundle: String::new(),
        admitted_transport: String::new(),
        endpoint_provenance: "hook".to_string(),
        child_pid: None,
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
        busy_seconds: 0,
        created_at: 1000,
        last_seen: 0,
        turn_started_at: 0,
        seen_cursor: 0,
        title: String::new(),
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
        attachment_dir: String::new(),
    };

    // No whitelist → the sender is treated as another agent. With no cached slug
    // the name falls back to the short sender pubkey ("pk-sende").
    let store = crate::state::Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("proj").metadata("proj", "", "", 1)
    ]));
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
        relay_event("abcdef123456", 9, "pk-sender", "proj", &row.body, 100, "[]"),
        relay_event(
            "rx-1",
            7,
            "pk-target",
            "proj",
            "👍",
            110,
            r#"[["e","abcdef123456"]]"#,
        ),
    ]));
    let prompt = crate::injection::render_terminal_mention(
        &store,
        &[row],
        &Default::default(),
        &[],
        120,
        false,
    )
    .unwrap();

    assert_eq!(
        prompt,
        "<mosaico>\n\
         \u{20}\u{20}<channel ref=\"#proj\">\n\
         \u{20}\u{20}\u{20}\u{20}<message from=\"@pk-sende\" id=\"abcdef\" age=\"20s\">please review the PTY delivery path</message>\n\
         \u{20}\u{20}</channel>\n\
         </mosaico>"
    );
}

#[test]
fn bounded_sender_name_is_carried_into_the_same_prompt_render() {
    let row = crate::state::InboxRow {
        event_id: "abcdef123456".into(),
        target_pubkey: "pk-target".into(),
        state: "pending".into(),
        from_pubkey: "pk-sender".into(),
        channel_h: "proj".into(),
        body: "please review".into(),
        created_at: 100,
        delivered_at: 0,
        attachment_dir: String::new(),
    };
    let store = crate::state::Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("proj").metadata("proj", "", "", 1)
    ]));
    let resolved_names = std::collections::BTreeMap::from([(
        "pk-sender".to_string(),
        "willow-echo-042-codex".to_string(),
    )]);

    let prompt =
        crate::injection::render_terminal_mention(&store, &[row], &resolved_names, &[], 120, false)
            .unwrap();

    assert!(
        prompt.contains("<message from=\"@willow-echo-042-codex\""),
        "{prompt}"
    );
}

#[test]
fn attachment_prompt_uses_one_directory_attribute_and_ordinary_bracket_labels() {
    let rec = sample_session();
    let row = crate::state::InboxRow {
        event_id: "abcdef123456".into(),
        target_pubkey: rec.pubkey,
        state: "pending".into(),
        from_pubkey: "pk-sender".into(),
        channel_h: "proj".into(),
        body: "Review [plan/report.md]".into(),
        created_at: 100,
        delivered_at: 0,
        attachment_dir: "/tmp/mosaico-files/abcdef".into(),
    };
    let store = crate::state::Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("proj").metadata("proj", "", "", 1)
    ]));

    let prompt = crate::injection::render_terminal_mention(
        &store,
        &[row],
        &Default::default(),
        &[],
        120,
        true,
    )
    .unwrap();

    assert!(prompt.contains("attachment-dir=\"/tmp/mosaico-files/abcdef\""));
    assert!(prompt.contains("Review [plan/report.md]"));
    assert!(!prompt.contains("<attachment"));
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
        attachment_dir: String::new(),
    };
    let store = crate::state::Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("mosaico").metadata("mosaico", "", "", 1),
        TestGroup::new("channel-writer-test").metadata("writer-test", "", "mosaico", 100),
    ]));
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
        relay_event(
            "ev-human",
            9,
            "human-pk",
            "channel-writer-test",
            &row.body,
            100,
            "[]",
        ),
        relay_event(
            "rx-2",
            7,
            rec.pubkey.as_str(),
            "channel-writer-test",
            "👍",
            110,
            r#"[["e","ev-human"]]"#,
        ),
    ]));
    // Sender is whitelisted, but the injected line still carries the source room.
    let prompt = crate::injection::render_terminal_mention(
        &store,
        &[row],
        &Default::default(),
        &["human-pk".into()],
        120,
        false,
    )
    .unwrap();
    assert_eq!(
        prompt,
        "<mosaico>\n\
         \u{20}\u{20}<channel ref=\"#mosaico/writer-test\">\n\
         \u{20}\u{20}\u{20}\u{20}<message from=\"@human-pk\" id=\"ev-hum\" age=\"20s\">@developer hey there</message>\n\
         \u{20}\u{20}</channel>\n\
         </mosaico>"
    );
}

#[test]
fn pending_mention_prompt_shows_coordination_guide_nudge() {
    let rec = sample_session();
    let row = crate::state::InboxRow {
        event_id: "abcdef123456".into(),
        target_pubkey: rec.pubkey.clone(),
        state: "pending".into(),
        from_pubkey: "pk-sender".into(),
        channel_h: "proj".into(),
        body: "please review the PTY delivery path".into(),
        created_at: 100,
        delivered_at: 0,
        attachment_dir: String::new(),
    };

    let store = crate::state::Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("proj").metadata("proj", "", "", 1)
    ]));
    let prompt = crate::injection::render_terminal_mention(
        &store,
        &[row],
        &Default::default(),
        &[],
        120,
        true,
    )
    .unwrap();

    assert!(
        prompt.contains("Follow up on abcdef: reply for substantive context or react for an ACK."),
        "{prompt}"
    );
    assert!(
        prompt.contains(
            "Read Mosaico's skill resource \
             `~/.agents/skills/mosaico/references/coordination-guide.md`"
        ),
        "{prompt}"
    );
}

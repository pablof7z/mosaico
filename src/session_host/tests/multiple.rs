use super::*;

#[test]
fn multiple_whitelisted_humans_render_as_distinct_named_senders() {
    let store = crate::state::Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("workspace").metadata("workspace", "", "", 1)
    ]));
    let humans = [
        ("pk-pablo", "Pablo", "PABLO-TOKEN"),
        ("pk-alice", "Alice", "ALICE-TOKEN"),
        ("pk-bob", "Bob", "BOB-TOKEN"),
    ];
    let profiles = humans
        .iter()
        .map(|(pubkey, name, _)| Profile {
            pubkey: (*pubkey).into(),
            name: (*name).into(),
            slug: (*name).into(),
            agent_slug: String::new(),
            host: String::new(),
            is_backend: false,
            agents: Vec::new(),
            workspaces: Vec::new(),
            updated_at: 100,
        })
        .collect::<Vec<_>>();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles(profiles));
    let rows = humans
        .iter()
        .enumerate()
        .map(|(index, (pubkey, _name, token))| crate::state::InboxRow {
            event_id: format!("event-{index}"),
            target_pubkey: "pk-target".into(),
            state: "pending".into(),
            from_pubkey: (*pubkey).into(),
            channel_h: "workspace".into(),
            body: (*token).into(),
            created_at: 100,
            delivered_at: 0,
            attachment_dir: String::new(),
        })
        .collect::<Vec<_>>();
    let whitelist = humans
        .iter()
        .map(|(pubkey, _, _)| (*pubkey).to_string())
        .collect::<Vec<_>>();

    let prompt = crate::injection::render_terminal_mention(
        &store,
        &rows,
        &Default::default(),
        &whitelist,
        120,
        false,
    )
    .expect("multi-human prompt");
    for (_, name, token) in humans {
        assert!(
            prompt.contains(&format!("<message from=\"@{name}\"")),
            "missing distinct sender label for {name}: {prompt}"
        );
        assert!(prompt.contains(token), "missing {token}: {prompt}");
    }
}

#[test]
fn multi_message_prompt_has_one_full_guide_and_one_compact_affordance_each() {
    let store = crate::state::Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("workspace").metadata("workspace", "", "", 1)
    ]));
    let rows = ["event-one", "event-two"]
        .into_iter()
        .map(|event_id| crate::state::InboxRow {
            event_id: event_id.into(),
            target_pubkey: "target".into(),
            state: "pending".into(),
            from_pubkey: "sender".into(),
            channel_h: "workspace".into(),
            body: "Please respond".into(),
            created_at: 100,
            delivered_at: 0,
            attachment_dir: String::new(),
        })
        .collect::<Vec<_>>();

    let prompt = crate::injection::render_terminal_mention(
        &store,
        &rows,
        &Default::default(),
        &[],
        120,
        true,
    )
    .unwrap();
    assert_eq!(prompt.matches("Follow up on ").count(), 2, "{prompt}");
    assert_eq!(
        prompt
            .matches(crate::reconcile::COORDINATION_GUIDE_REMINDER)
            .count(),
        1,
        "{prompt}"
    );
}

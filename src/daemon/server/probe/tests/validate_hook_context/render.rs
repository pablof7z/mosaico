use super::*;

#[tokio::test]
async fn rpc_probe_validate_hook_target_fails_unconfirmed_rendered_channel() {
    let state = DaemonState::new_for_test().await;
    seed_session_row(&state, "s1", "ghost", false);
    seed_hook_graph_and_receipt_with_inputs(&state, "s1", unconfirmed_channel_inputs(), None);

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "hook:s1" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "hook_context_outcome", "failed");
    assert_eq!(
        v["hook_context_evidence"]["graph"]["rendered_unconfirmed_channel"],
        true
    );
    assert_eq!(
        v["hook_context_evidence"]["session_channel"]["confirmed"],
        false
    );
    assert!(v["hook_context_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("degraded warnings"));
}

#[tokio::test]
async fn rpc_probe_validate_hook_target_reports_local_agents_separate_from_members() {
    let state = DaemonState::new_for_test().await;
    seed_session_row(&state, "s1", "room", true);
    seed_hook_graph_and_receipt_with_inputs(&state, "s1", local_agents_and_members_inputs(), None);

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "hook:s1" })).unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "hook_context_outcome", "passed");
    assert_eq!(
        v["hook_context_evidence"]["graph"]["rendered_local_agents"],
        true
    );
    assert_eq!(
        v["hook_context_evidence"]["graph"]["rendered_member_roster"],
        true
    );
    assert_eq!(
        v["hook_context_evidence"]["member_roster_corroborated"],
        true
    );
    assert_eq!(
        v["hook_context_evidence"]["session_channel"]["membership_snapshot"],
        true
    );
    assert_eq!(
        v["hook_context_evidence"]["graph"]["rendered_legacy_agents_roster"],
        false
    );
    assert_eq!(v["hook_context_evidence"]["graph"]["local_agent_rows"], 1);
    assert_eq!(v["hook_context_evidence"]["graph"]["member_rows"], 2);
}

#[tokio::test]
async fn rpc_probe_validate_hook_target_fails_member_roster_without_snapshot() {
    let state = DaemonState::new_for_test().await;
    seed_session_channel_without_roster(&state, "s1", "room");
    seed_hook_graph_and_receipt_with_inputs(&state, "s1", local_agents_and_members_inputs(), None);

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "hook:s1" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "hook_context_outcome", "failed");
    assert_eq!(
        v["hook_context_evidence"]["member_roster_corroborated"],
        false
    );
    assert_eq!(
        v["hook_context_evidence"]["session_channel"]["membership_snapshot"],
        false
    );
    assert!(v["hook_context_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("membership snapshot"));
}

#[tokio::test]
async fn rpc_probe_validate_hook_target_fails_legacy_agents_roster() {
    let state = DaemonState::new_for_test().await;
    seed_session_row(&state, "s1", "room", true);
    seed_hook_graph_and_receipt_with_inputs(&state, "s1", local_agents_and_members_inputs(), None);
    state
        .hook_contexts
        .lock()
        .unwrap()
        .get_mut("s1")
        .unwrap()
        .set_current_text_for_test(
            r#"<tenex-edge><channel name="room"><agents><agent ref="@helper" /></agents></channel></tenex-edge>"#,
        );

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "hook:s1" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "hook_context_outcome", "failed");
    assert_eq!(
        v["hook_context_evidence"]["graph"]["rendered_legacy_agents_roster"],
        true
    );
    assert!(v["hook_context_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("available-agents"));
}

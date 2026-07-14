use super::*;

#[tokio::test]
async fn rpc_probe_validate_accepts_visible_cause_labels() {
    let state = DaemonState::new_for_test().await;
    seed_visible_path_state(&state);

    let cases = [
        (
            "subscriptions/daemon/channels",
            "subscriptions",
            "subscription cause labels identify Trellis inputs",
        ),
        (
            "planner: subscriptions/daemon/subs",
            "subscriptions",
            "planner collections",
        ),
        (
            "planner: status/s1/coll",
            "status",
            "planner labels name Trellis nodes/collections",
        ),
        (
            "session_watch/resources",
            "session_watch",
            "session_watch cause labels identify Trellis graph inputs",
        ),
    ];

    for (target, surface, reason) in cases {
        let v = rpc_probe(&state, &json!({ "verb": "validate", "target": target })).unwrap();
        assert_eq!(v["surface"], surface);
        assert!(v["target_evidence"].is_null());
        assert_check_status(&v, "cause_label", "passed");
        assert_eq!(v["cause_label_evidence"]["surface"], surface);
        assert!(v["cause_label_evidence"]["reason"]
            .as_str()
            .unwrap()
            .contains(reason));
    }
}

#[tokio::test]
async fn rpc_probe_validate_reports_malformed_explain_handles_inside_envelope() {
    let state = DaemonState::new_for_test().await;

    let cases = [
        ("event:", "event handle id must be non-empty"),
        ("llm:not-an-id", "llm handle id must be an integer"),
        ("txn::1", "txn handle surface must be non-empty"),
        ("txn:not-a-surface:1", "not a known validation surface"),
        ("txn:status:not-an-id", "txn id must be an integer"),
        ("txn:status:1@soon", "@<ts> must be unix millis"),
        ("session:s1@soon", "@<ts> must be unix millis"),
    ];

    for (target, reason) in cases {
        let v = rpc_probe(&state, &json!({ "verb": "validate", "target": target })).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["verdict"], "failed");
        assert!(v["explain"].is_null());
        assert!(v["state"].is_null());
        assert_check_status(&v, "target", "failed");
        assert_no_check(&v, "explain");
        assert_no_check(&v, "state");
        assert_eq!(v["target_evidence"]["kind"], "invalid_explain_handle");
        assert_eq!(v["target_evidence"]["valid"], false);
        assert!(v["target_evidence"]["reason"]
            .as_str()
            .unwrap()
            .contains(reason));
    }
}

#[tokio::test]
async fn rpc_probe_validate_reports_empty_known_handles_inside_envelope() {
    let state = DaemonState::new_for_test().await;

    let cases = [
        ("sub:", "subscription channel"),
        ("status/", "status resource"),
        ("outbox/", "outbox resource"),
        ("session:", "session pubkey"),
        ("session-watch/", "session_watch resource"),
    ];

    for (target, reason) in cases {
        let v = rpc_probe(&state, &json!({ "verb": "validate", "target": target })).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["verdict"], "failed");
        assert!(v["why"].is_null());
        assert!(v["state"].is_null());
        assert_check_status(&v, "target", "failed");
        assert_no_check(&v, "state");
        assert_eq!(v["target_evidence"]["kind"], "empty_handle");
        assert_eq!(v["target_evidence"]["valid"], false);
        assert!(v["target_evidence"]["reason"]
            .as_str()
            .unwrap()
            .contains(reason));
    }
}

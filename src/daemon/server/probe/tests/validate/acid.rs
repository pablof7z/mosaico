use super::*;

#[tokio::test]
async fn rpc_probe_validate_uses_arm_cause_for_status_refresh() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.status.lock().expect("status mutex");
        r.on_session_started(
            "s1",
            "laptop",
            "coder",
            ".",
            BTreeSet::from(["room".to_string()]),
            false,
            "T",
            "",
            100,
        )
        .unwrap();
        r.on_tick("s1", 130).unwrap();
    }

    let validation = rpc_probe(
        &state,
        &json!({
            "verb": "validate",
            "target": "status:s1",
            "fact": {
                "StatusDrive": {
                    "DistillCompleted": {
                        "pubkey": "s1",
                        "title": "T",
                        "activity": "hidden while idle",
                        "window_hash": "sha256:w",
                        "at": 160
                    }
                }
            },
            "since": 0
        }),
    )
    .unwrap();

    assert_check_status(&validation, "acid", "passed");
    assert_eq!(validation["acid"]["cause"], "status/s1/arm");
}

#[tokio::test]
async fn rpc_probe_validate_reports_acid_errors_inside_envelope() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.status.lock().expect("status mutex");
        r.on_session_started(
            "s1",
            "laptop",
            "coder",
            ".",
            BTreeSet::from(["room".to_string()]),
            false,
            "T",
            "",
            100,
        )
        .unwrap();
        r.on_tick("s1", 130).unwrap();
    }

    let validation = rpc_probe(
        &state,
        &json!({
            "verb": "validate",
            "target": "status:s1",
            "fact": {
                "StatusDrive": {
                    "Tick": {
                        "pubkey": "s1",
                        "at": 160
                    }
                }
            }
        }),
    )
    .unwrap();

    assert_eq!(validation["ok"], true);
    assert_eq!(validation["verdict"], "passed_with_limitations");
    assert_check_status(&validation, "acid", "not_proven");
    assert!(validation["acid"].is_null());
    assert!(validation["acid_error"]
        .as_str()
        .unwrap()
        .contains("no unrelated mutation"));
}

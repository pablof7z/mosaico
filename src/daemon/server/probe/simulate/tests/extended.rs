use super::*;

#[tokio::test]
async fn simulate_session_start_accepts_input_fact_json_without_mutating() {
    let state = DaemonState::new_for_test().await;
    let fact = InputFact::SessionStartRequested(session_start_request());

    let out = simulate_value(&state, &json!({ "verb": "simulate", "fact": fact })).unwrap();

    assert_eq!(out["surface"], "session_start");
    assert_eq!(out["would_effect"], true);
    assert_eq!(out["commands"][0]["op"], "Open");
    assert!(out["changed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "session_start/s1/request"));
    assert_eq!(out["revision_before"], out["revision_after"]);
}

#[tokio::test]
async fn simulate_hook_context_accepts_input_fact_json_without_mutating() {
    let state = DaemonState::new_for_test().await;
    seed_hook_context_graph(&state);
    let fact = InputFact::HookContextRender(HookContextRenderFact {
        pubkey: "s1".into(),
        hook_kind: "turn_start".into(),
        cursor: 0,
        now: 100,
        force: false,
        emitted_text_hash: None,
        inputs_json: hook_inputs_json(&["probe warning"]),
    });

    let out = simulate_value(&state, &json!({ "verb": "simulate", "fact": fact })).unwrap();

    assert_eq!(out["surface"], "hook_context");
    assert_eq!(out["would_effect"], true);
    assert_eq!(out["output_frames"], 1);
    assert!(out["changed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .all(|label| !label.starts_with("node:")));
    assert_eq!(out["revision_before"], out["revision_after"]);
}

#[tokio::test]
async fn simulate_unowned_facts_returns_explanation_without_erroring() {
    let state = DaemonState::new_for_test().await;
    let cases = vec![
        (
            InputFact::RelayEventObserved {
                event_id: "ev1".into(),
                kind: 1,
                channel_h: Some("room".into()),
                pubkey: "pk".into(),
                at: 100,
            },
            "event_ingest",
        ),
        (InputFact::ClockTick { at: 102 }, "timekeeping"),
    ];

    for (fact, frontier) in cases {
        let out = simulate_value(&state, &json!({ "verb": "simulate", "fact": fact })).unwrap();
        assert_eq!(out["simulated"], false);
        assert_eq!(out["ok"], false);
        assert_eq!(out["would_effect"], false);
        assert_eq!(out["fact_evidence"]["frontier"], frontier);
        assert!(out["fact_evidence"]["reason"]
            .as_str()
            .unwrap()
            .contains("no"));
    }
}

#[tokio::test]
async fn simulate_process_exit_closes_session_watch_without_mutating() {
    let state = DaemonState::new_for_test().await;
    state
        .session_watch
        .lock()
        .unwrap()
        .apply(&InputFact::SessionStarted {
            pubkey: "s1".into(),
            channel_h: Some("room".into()),
            pid: Some(42),
            at: 100,
        })
        .unwrap();
    let before_rev = state.session_watch.lock().unwrap().revision();
    let fact = InputFact::ProcessExited {
        pubkey: Some("s1".into()),
        pid: 42,
        at: 101,
    };

    let out = simulate_value(&state, &json!({ "verb": "simulate", "fact": fact })).unwrap();

    assert_eq!(out["surface"], "session_watch");
    assert_eq!(out["would_effect"], true);
    assert_eq!(out["commands"][0]["op"], "Close");
    assert!(out["changed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "session_watch/live_sessions"));
    assert_eq!(out["revision_before"], out["revision_after"]);
    assert_eq!(state.session_watch.lock().unwrap().revision(), before_rev);
}

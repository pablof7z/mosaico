use super::*;
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_alias_resolves_live_consistent_session() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1", "room", true);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "alias:codex:harness_session:native-s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "alias", "passed");
    assert_eq!(v["alias_evidence"]["resolved_session_id"], "s1");
    assert_eq!(v["alias_evidence"]["resolved_live"], true);
    assert_eq!(v["alias_evidence"]["status_found"], true);
    assert_eq!(v["alias_evidence"]["watch_found"], true);
}

#[tokio::test]
async fn rpc_probe_validate_alias_supports_pty_session_shorthand() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1", "room", true);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "pty_session:%1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "alias", "passed");
    assert_eq!(v["alias_evidence"]["alias_kind"], "pty_session");
    assert_eq!(v["alias_evidence"]["harness"], serde_json::Value::Null);
    assert_eq!(v["alias_evidence"]["resolved_session_id"], "s1");
}

#[tokio::test]
async fn rpc_probe_validate_alias_reports_dead_alias_as_not_proven() {
    let state = DaemonState::new_for_test().await;
    seed_alive_session(&state, "s1", "room", false);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "harness_session:codex:native-s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "alias", "not_proven");
    assert_eq!(v["alias_evidence"]["found"], true);
    assert_eq!(v["alias_evidence"]["resolved_live"], false);
    assert_eq!(v["alias_evidence"]["session_alive"], false);
}

#[tokio::test]
async fn rpc_probe_validate_alias_missing_is_not_proven() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "watch_pid:999999" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "alias", "not_proven");
    assert_eq!(v["alias_evidence"]["found"], false);
}

fn seed_alive_session(
    state: &std::sync::Arc<DaemonState>,
    session_id: &str,
    channel_h: &str,
    alive: bool,
) {
    state
        .with_store(|s| {
            s.upsert_session_row(
                session_id,
                &RegisterSession {
                    harness: "codex".into(),
                    external_id_kind: "harness_session".into(),
                    external_id: format!("native-{session_id}"),
                    agent_pubkey: "pk1".into(),
                    agent_slug: "codex".into(),
                    channel_h: channel_h.into(),
                    child_pid: Some(123),
                    transcript_path: None,
                    resume_id: "resume-s1".into(),
                    now: 100,
                },
            )?;
            s.put_alias(
                "codex",
                "harness_session",
                &format!("native-{session_id}"),
                session_id,
                100,
            )?;
            s.put_alias("codex", "pty_session", "%1", session_id, 100)?;
            s.put_alias("codex", "watch_pid", "123", session_id, 100)?;
            if !alive {
                s.mark_dead(session_id)?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
    if alive {
        seed_status_graph(state, session_id, channel_h);
        seed_subscription_graph(state, session_id, channel_h);
        seed_session_watch_graph(state, session_id, channel_h);
    }
}

fn seed_status_graph(state: &std::sync::Arc<DaemonState>, session_id: &str, channel_h: &str) {
    state
        .status
        .lock()
        .unwrap()
        .on_session_started(
            session_id,
            "laptop",
            "codex",
            "pk1",
            ".",
            BTreeSet::from([channel_h.to_string()]),
            false,
            "T",
            "",
            100,
        )
        .unwrap();
}

fn seed_subscription_graph(state: &std::sync::Arc<DaemonState>, session_id: &str, channel_h: &str) {
    let mut sessions = BTreeMap::new();
    sessions.insert(
        session_id.to_string(),
        BTreeSet::from([channel_h.to_string()]),
    );
    state
        .subs
        .lock()
        .unwrap()
        .sync(&CoverageSnapshot {
            daemon_channels: BTreeSet::new(),
            addressed_pubkeys: BTreeSet::new(),
            archived_channels: BTreeSet::new(),
            sessions,
        })
        .unwrap();
}

fn seed_session_watch_graph(
    state: &std::sync::Arc<DaemonState>,
    session_id: &str,
    channel_h: &str,
) {
    state
        .session_watch
        .lock()
        .unwrap()
        .apply(&InputFact::SessionStarted {
            session_id: session_id.into(),
            channel_h: Some(channel_h.into()),
            agent_pubkey: Some("pk1".into()),
            pid: Some(123),
            at: 100,
        })
        .unwrap();
}

fn assert_check_status(v: &serde_json::Value, name: &str, status: &str) {
    assert_eq!(check_row(v, name)["status"], status);
}

fn check_row<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .expect("check row")
}

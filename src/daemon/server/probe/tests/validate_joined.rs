use super::*;
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_joined_channel_passes_with_subscription_coverage() {
    let state = DaemonState::new_for_test().await;
    seed_session_with_joined_channels(&state, "s1", &["room", "side"]);
    seed_subscription_graph(&state, "s1", &["room", "side"]);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "joined:s1:side" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "joined_channels", "passed");
    assert_eq!(v["joined_evidence"]["requested_joined"], true);
    assert_eq!(v["joined_evidence"]["missing_subscription_count"], 0);
}

#[tokio::test]
async fn rpc_probe_validate_joined_channel_fails_missing_subscription_coverage() {
    let state = DaemonState::new_for_test().await;
    seed_session_with_joined_channels(&state, "s1", &["room", "side"]);
    seed_subscription_graph(&state, "s1", &["room"]);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "joined/s1/side" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "joined_channels", "failed");
    assert_eq!(v["joined_evidence"]["requested_joined"], true);
    assert_eq!(v["joined_evidence"]["missing_subscription_count"], 1);
}

#[tokio::test]
async fn rpc_probe_validate_joined_channel_reports_unjoined_channel() {
    let state = DaemonState::new_for_test().await;
    seed_session_with_joined_channels(&state, "s1", &["room"]);
    seed_subscription_graph(&state, "s1", &["room"]);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "session_channel:s1:side" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "joined_channels", "not_proven");
    assert_eq!(v["joined_evidence"]["requested_joined"], false);
}

fn seed_session_with_joined_channels(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    channels: &[&str],
) {
    let active = channels.first().copied().unwrap_or("room");
    state
        .with_store(|s| {
            for channel in channels {
                s.upsert_channel(channel, channel, "", "", 100)?;
            }
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                pubkey: pubkey.into(),
                agent_slug: "coder".into(),
                channel_h: active.into(),
                child_pid: None,
                transcript_path: None,
                now: 100,
            })?;
            for (idx, channel) in channels.iter().enumerate() {
                s.join_session_channel(pubkey, channel, 100 + idx as u64)?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

fn seed_subscription_graph(state: &std::sync::Arc<DaemonState>, pubkey: &str, channels: &[&str]) {
    let mut sessions = BTreeMap::new();
    sessions.insert(
        pubkey.to_string(),
        channels.iter().map(|channel| channel.to_string()).collect(),
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

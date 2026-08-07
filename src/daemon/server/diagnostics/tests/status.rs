use super::*;
use crate::reconcile::{PresenceProjection, PresenceSnapshot};
use crate::session_state::SessionState;
use std::collections::BTreeSet;

#[tokio::test]
async fn status_receipt_reaches_actual_doctor_rpc_json() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    let keys = Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let management = state.backend_pubkey().unwrap();
    // The publish gate reads the relay-signed roster from the cache the
    // retained group-records observation keeps current, so the fixture seeds
    // the cache rather than scripting a bounded read that no longer happens.
    state
        .with_store(|store| {
            store.upsert_channel("project", "project", "", "", 1)?;
            store.replace_channel_admins("project", std::slice::from_ref(&management), 2)?;
            store.replace_channel_members("project", std::slice::from_ref(&pubkey), 3)
        })
        .unwrap();

    let now = crate::util::now_secs();
    crate::presence_publisher::drive(
        &state.reconcilers.status,
        &state.reconcilers.presence_publisher,
        &keys,
        crate::presence_publisher::DriveMeta {
            trigger: "doctor-corpus",
        },
        |status| {
            status.open(
                &pubkey,
                1,
                PresenceSnapshot {
                    host: "test-host".into(),
                    workspace: "project".into(),
                    slug: "status-corpus".into(),
                    rel_cwd: ".".into(),
                    dispatch_event: None,
                    projection: PresenceProjection {
                        channels: BTreeSet::from(["project".into()]),
                        branch: "fix/701-operator-visibility".into(),
                        state: SessionState::Working,
                        state_since: now,
                        title: "Proving status receipt provenance".into(),
                    },
                },
                now,
            )
        },
    );
    let source_ref = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let artifact = state
                .with_store(|store| store.latest_receipts_for_surface("status", 1))
                .unwrap()
                .into_iter()
                .next()
                .and_then(|receipt| receipt.artifact_ref);
            if let Some(artifact) = artifact {
                break artifact;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("presence publisher status receipt");

    state.nmp().script_read_events(Vec::new());
    let response = super::super::super::dispatch(
        &state,
        &Request {
            id: 705,
            method: "doctor".into(),
            params: serde_json::json!({}),
        },
    )
    .await;
    let json = response.ok.expect("doctor RPC response");
    // The presence publisher recorded an id it got from NMP, and the doctor's
    // account of what this daemon still owes names the SAME write. The two
    // used to be able to disagree, because the id was derived by Mosaico and
    // the evidence came from a process-local observer beside NMP's queue.
    let queue = &json["publish_queue"];
    assert!(queue["outstanding"].as_u64().unwrap() >= 1, "{queue}");
    let stuck = queue["stuck"].as_array().unwrap();
    assert!(stuck.is_empty(), "a status write needs nobody: {queue}");
    let entries = state
        .nmp()
        .publish_queue_entry_ids()
        .expect("the publish queue is readable");
    assert!(
        entries.contains(&source_ref),
        "the id the presence publisher recorded ({source_ref}) is not one NMP froze: {entries:?}"
    );
    eprintln!(
        "CORPUS_STATUS_DOCTOR_JSON={}",
        serde_json::to_string(&json).unwrap()
    );
}

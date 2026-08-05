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
    state
        .nmp
        .script_write_facts(vec![WriteFact::Signing(SigningState::Refused {
            reason: SCRIPTED_CLASSIFIED_FAILURE.into(),
        })]);

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
    state.nmp.wait_background_receipts();

    state.nmp.script_read_events(Vec::new());
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
    let failure = &json["background_writes"]["last_failure"];
    assert_eq!(failure["status"], "signer_refused");
    assert_eq!(failure["operation"], "status");
    assert_eq!(failure["source_ref"], source_ref);
    // One group write is now ONE intent routed to the whole scope, so the
    // stream that reports on it is named by the group rather than by an index
    // into a per-relay fan-out Mosaico no longer performs.
    assert_eq!(failure["target"], "every group host");
    assert_eq!(failure["detail"], SCRIPTED_CLASSIFIED_FAILURE);
    eprintln!(
        "CORPUS_STATUS_DOCTOR_JSON={}",
        serde_json::to_string(&json).unwrap()
    );
}

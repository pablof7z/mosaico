use super::*;
use crate::reconcile::{PresenceProjection, PresenceSnapshot};
use crate::session_state::SessionState;
use std::collections::BTreeSet;

#[tokio::test]
async fn status_receipt_names_the_exact_write_nmp_froze() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    let keys = Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let management = state.backend_pubkey().unwrap();
    // The publish gate reads the relay-signed roster from the cache the
    // retained group-records observation keeps current, so the fixture seeds
    // the cache rather than scripting a bounded read that no longer happens.
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("project")
                    .metadata("project", "", "", 1)
                    .admins(vec![management.clone()])
                    .members(vec![pubkey.clone()]),
            ]));
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();

    let now = crate::util::now_secs();
    crate::presence_publisher::drive(
        &state.reconcilers.status,
        &state.reconcilers.presence_publisher,
        &keys,
        crate::presence_publisher::DriveMeta {
            trigger: "receipt-corpus",
            confirmed_scope: None,
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

    // The presence publisher records the id NMP returned at durable
    // acceptance. It must be the exact frozen queue entry, never a local
    // recomputation of the Nostr event id.
    let entries = state
        .nmp()
        .publish_queue_entry_ids()
        .expect("the publish queue is readable");
    assert!(
        entries.contains(&source_ref),
        "the id the presence publisher recorded ({source_ref}) is not one NMP froze: {entries:?}"
    );
    eprintln!("CORPUS_STATUS_RECEIPT={source_ref}");
}

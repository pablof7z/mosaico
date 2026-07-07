use super::*;

/// The proactive-warm selection: already-named identities are skipped (no
/// network), empty pubkeys are ignored, and a pubkey already in flight is not
/// re-claimed, so duplicate relay deliveries collapse to one fetch.
#[tokio::test]
async fn claim_skips_known_empty_and_in_flight() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|s| {
        s.upsert_profile("known-pk", "pablo", "pablo", "laptop", false, 1)
            .unwrap();
    });

    let claimed = claim_pubkeys_to_warm(
        &state,
        vec!["known-pk".into(), "new-pk".into(), String::new()],
    );
    assert_eq!(
        claimed,
        vec!["new-pk".to_string()],
        "only the uncached, non-empty pubkey is claimed for a fetch"
    );

    let again = claim_pubkeys_to_warm(&state, vec!["new-pk".into()]);
    assert!(again.is_empty(), "an in-flight pubkey is not re-claimed");
}

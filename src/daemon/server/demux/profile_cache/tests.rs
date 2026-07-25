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

/// The corrective refetch is driven by the render path, so the same unresolvable
/// member arrives on every turn. Within one debounce window only the first
/// arrival is claimed.
#[tokio::test]
async fn refetch_claims_a_pubkey_once_per_debounce_window() {
    let state = DaemonState::new_for_test().await;

    let first = claim_profiles_to_refetch(&state, vec!["ghost".into(), "wraith".into()], 1_000);
    assert_eq!(first, vec!["ghost".to_string(), "wraith".to_string()]);

    // A turn one second later surfaces the same withheld members again.
    let second = claim_profiles_to_refetch(&state, vec!["ghost".into(), "wraith".into()], 1_001);
    assert!(
        second.is_empty(),
        "a turn inside the window must not re-fetch: {second:?}"
    );

    // A member that was not withheld before is still claimed immediately, even
    // though its neighbours are mid-window.
    let third = claim_profiles_to_refetch(&state, vec!["ghost".into(), "newcomer".into()], 1_002);
    assert_eq!(third, vec!["newcomer".to_string()]);
}

/// Past the window the same pubkey is retried — a `kind:0` that was simply not
/// on the relays yet must not be written off permanently.
#[tokio::test]
async fn refetch_retries_after_the_debounce_window_elapses() {
    let state = DaemonState::new_for_test().await;

    assert_eq!(
        claim_profiles_to_refetch(&state, vec!["ghost".into()], 1_000),
        vec!["ghost".to_string()]
    );
    assert!(claim_profiles_to_refetch(
        &state,
        vec!["ghost".into()],
        1_000 + PROFILE_REFETCH_DEBOUNCE_SECS - 1
    )
    .is_empty());
    assert_eq!(
        claim_profiles_to_refetch(
            &state,
            vec!["ghost".into()],
            1_000 + PROFILE_REFETCH_DEBOUNCE_SECS
        ),
        vec!["ghost".to_string()],
        "the pubkey is eligible again once the window has passed"
    );
}

/// The attempt ledger is pruned as it is consulted, so it stays bounded by the
/// pubkeys seen in one window rather than accumulating every peer forever.
#[tokio::test]
async fn refetch_ledger_forgets_attempts_older_than_the_window() {
    let state = DaemonState::new_for_test().await;

    claim_profiles_to_refetch(&state, vec!["ghost".into(), "wraith".into()], 1_000);
    assert_eq!(
        state.dedup.profile_refetch_attempts.lock().unwrap().len(),
        2
    );

    claim_profiles_to_refetch(
        &state,
        vec!["newcomer".into()],
        1_000 + PROFILE_REFETCH_DEBOUNCE_SECS,
    );
    let ledger = state.dedup.profile_refetch_attempts.lock().unwrap();
    assert_eq!(
        ledger.keys().collect::<Vec<_>>(),
        vec!["newcomer"],
        "expired attempts are dropped rather than retained"
    );
}

/// Empty input never touches the shared ledger, and an empty pubkey is never a
/// fetch target.
#[tokio::test]
async fn refetch_ignores_empty_input_and_empty_pubkeys() {
    let state = DaemonState::new_for_test().await;

    assert!(claim_profiles_to_refetch(&state, Vec::new(), 1_000).is_empty());
    assert!(state
        .dedup
        .profile_refetch_attempts
        .lock()
        .unwrap()
        .is_empty());

    assert!(claim_profiles_to_refetch(&state, vec![String::new()], 1_000).is_empty());
    assert!(state
        .dedup
        .profile_refetch_attempts
        .lock()
        .unwrap()
        .is_empty());
}

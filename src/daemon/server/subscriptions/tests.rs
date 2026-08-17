//! Production-path proof for the retained profile feed (mosaico#837, slice 1).
//!
//! Drives the daemon's real coverage refresh (`sync_subscriptions`) with live
//! group state, delivers a member's kind:0 through a real in-process relay +
//! the NMP engine, and asserts `Store::get_profile` — the production read,
//! NOT the `install_test_profiles` seam — resolves it.

use std::time::{Duration, Instant};

use nostr::Keys;

use crate::daemon::server::DaemonState;
use crate::domain::{AgentRef, DomainEvent, Profile as DomainProfile};
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::nmp_host::PlainRelay;
use crate::state::{Store, TestGroup, TestGroupDelivery};

use super::sync_subscriptions;

/// One non-backend kind:0 signed by `keys` carrying `name` as its metadata.
fn kind0_event(keys: &Keys, name: &str, host: &str, created_at: u64) -> nostr::Event {
    let profile = DomainProfile {
        agent: AgentRef::new(keys.public_key().to_hex(), name),
        agent_slug: String::new(),
        host: host.to_string(),
        workspace: "ws".to_string(),
        owners: Vec::new(),
        is_backend: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
    };
    Nip29WireCodec
        .encode_event(&DomainEvent::Profile(profile))
        .expect("encode kind:0")
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(keys)
        .expect("sign kind:0")
}

/// The production path: group state drives `set_members`, the feed's retained
/// `authors:[members]` observation drains a real kind:0, and `Store::get_profile`
/// returns it. No `install_test_profiles` is used — the feed is populated by the
/// live NMP delivery alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_get_profile_reads_the_live_feed_driven_by_membership() {
    let member = Keys::generate();
    let pk = member.public_key().to_hex();
    let relay = PlainRelay::spawn([kind0_event(&member, "live-member", "laptop", 100)]);
    let state = DaemonState::new_for_test_with_relays(vec![relay.url()]).await;

    // Drive group state so the coverage refresh computes a member set that
    // includes the member. `with_groups` reads this test seam, so
    // `build_coverage_snapshot` sees the member in `profile_pubkeys` and
    // `sync_subscriptions` calls `profile_feed.set_members({backend, member})`.
    state.with_store(|store: &Store| {
        store.install_test_nmp_group_delivery(TestGroupDelivery::new([TestGroup::new("room")
            .metadata("room", "", "", 1)
            .members([pk.clone()])]));
    });
    sync_subscriptions(&state)
        .await
        .expect("coverage refresh opens the feed observation");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let resolved = state.with_store(|store| store.get_profile(&pk).ok().flatten());
        if let Some(profile) = resolved {
            assert_eq!(
                profile.name, "live-member",
                "production get_profile must return the live-delivered profile"
            );
            assert_eq!(profile.pubkey, pk);
            break;
        }
        if Instant::now() >= deadline {
            panic!("production get_profile never resolved the live member profile");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    drop(state);
    relay.shutdown();
}
